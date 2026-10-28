import { describe, expect, it } from 'vitest';
import { setTimeout as delay } from 'timers/promises';
import fetch from 'node-fetch';
import { DASH0_ENDPOINT, DASH0_LAMBDA_TESTS_DATASET, DASH0_TOKEN, MAX_ATTEMPTS, RETRY_DELAY_MS, TEST_TIMEOUT_MS } from './config';
import { checkMainSpans, getRequestPayload, invokeFunction, RESOURCE_PREFIX } from './utils';

// Sent by every masking_in_extension handler. With DASH0_MASK_IN_EXTENSION=true the distros
// mask nothing, so a `****` in Dash0 can only come from the extension, and none of it may
// reach Dash0.
const SECRET = 'XQZ-IT-SECRET';
// `"key":"****"`, `key=****`, `\"key\":\"****\"` or Python's `'key': '****'`, whichever way the
// value was serialized.
const maskedKey = (key: string) => new RegExp(`${key}[\\\\"' ]*[:=][\\\\"' ]*\\*\\*\\*\\*`, 'i');

const query = async (signal: 'spans' | 'logs', body: object): Promise<any> => {
    const response = await fetch(DASH0_ENDPOINT + signal, {
        method: 'POST',
        headers: {
            accept: 'application/json',
            authorization: `Bearer ${DASH0_TOKEN}`,
            'content-type': 'application/json',
        },
        body: JSON.stringify(body),
    });
    return response.json();
};

/** Every span of the trace and every log record of the invocation, once the checks pass. */
const exported = async (
    functionName: string,
    invocationId: string,
    traceId: string,
    ready: (spans: any[], logs: any[]) => void,
): Promise<{ spans: any[]; logs: any[] }> => {
    const now = Date.now();
    for (let attempt = 1; ; attempt++) {
        await delay(RETRY_DELAY_MS);
        try {
            const spanPayload = await query('spans', {
                filter: [{ operator: 'is', key: 'service.name', value: functionName }],
                timeRange: {
                    from: new Date(now - 10 * 60_000).toISOString(),
                    to: new Date(now + 10 * 60_000).toISOString(),
                },
                sampling: { mode: 'adaptive' },
                dataset: DASH0_LAMBDA_TESTS_DATASET,
            });
            const spans = (spanPayload?.resourceSpans ?? [])
                .flatMap((rs: any) => rs.scopeSpans ?? [])
                .flatMap((ss: any) => ss.spans ?? [])
                .filter((s: any) => s.traceId === traceId);
            const logPayload = await query('logs', getRequestPayload(invocationId));
            const logs = (logPayload?.resourceLogs ?? [])
                .flatMap((rl: any) => rl.scopeLogs ?? [])
                .flatMap((sl: any) => sl.logRecords ?? []);
            ready(spans, logs);
            return { spans, logs };
        } catch (error) {
            console.error(`Attempt ${attempt} for ${functionName}:`, error);
            if (attempt === MAX_ATTEMPTS) {
                throw error;
            }
        }
    }
};

const attr = (span: any, key: string): string | undefined =>
    span.attributes?.find((a: any) => a.key === key)?.value?.stringValue;

const urlOf = (span: any): string | undefined => attr(span, 'url.full') ?? attr(span, 'http.url');

const requestBodyLog = (logs: any[]) =>
    logs.find((l) => JSON.stringify(l).includes('http_request_body'));

// Known gap: a URL inside another value isn't query-masked. The test server answers the POST with
// a `location` header that echoes the request URL, secret included (docs/masking.md, "Still not
// masked with this setting").
const withoutLocationHeader = (json: string) =>
    json.replace(/((?:\\"|')location(?:\\"|'):\s*(?:\\"|'))[^"'\\]*/gi, '$1<url>');

const assertNothingLeaked = (spans: any[], logs: any[]) => {
    for (const span of spans) {
        const attributes = withoutLocationHeader(JSON.stringify(span.attributes));
        expect(attributes, `span ${span.name} leaked the secret`).not.toContain(SECRET);
    }
    for (const log of logs) {
        expect(JSON.stringify(log), 'log record leaked the secret').not.toContain(SECRET);
    }
};

const cases: {
    family: string;
    handlerScopeName: string;
    ready: (spans: any[], logs: any[]) => void;
}[] = [
    {
        family: 'node',
        handlerScopeName: '@opentelemetry/instrumentation-aws-lambda',
        ready: (spans, logs) => {
            const http = spans.find((s) => urlOf(s)?.includes('api_token='));
            expect(http, 'HTTP client span').toBeDefined();
            expect(urlOf(http)).toContain('api_token=****&page=1');
            // The form body, written by the distro as a quoted string, masked by field name.
            const body = requestBodyLog(logs);
            expect(body, 'http_request_body log').toBeDefined();
            expect(JSON.stringify(body)).toMatch(maskedKey('password'));
            expect(JSON.stringify(body)).toContain('jane%40example.com');
            // The JSON body and Authorization header, which the distro no longer masks.
            const json = spans.find((s) => /^https:\/\/[^?]*\/posts$/.test(urlOf(s) ?? ''));
            expect(json, 'JSON POST span').toBeDefined();
            expect(attr(json, 'http.request.headers')).toMatch(maskedKey('authorization'));
        },
    },
    {
        family: 'python',
        handlerScopeName: 'opentelemetry.instrumentation.aws_lambda',
        ready: (spans, logs) => {
            const http = spans.find((s) => urlOf(s)?.includes('api_token='));
            expect(http, 'HTTP client span').toBeDefined();
            expect(urlOf(http)).toContain('api_token=****&page=1');
            expect(attr(http, 'http.request.headers')).toMatch(maskedKey('x-api-key'));
            const body = requestBodyLog(logs);
            expect(body, 'http_request_body log').toBeDefined();
            expect(JSON.stringify(body)).toMatch(maskedKey('password'));
            expect(JSON.stringify(body)).toContain('jane@example.com');
            // The AWS SDK request body, which the distro no longer masks.
            expect(JSON.stringify([spans, logs])).toMatch(maskedKey('apiToken'));
        },
    },
    {
        family: 'java',
        handlerScopeName: 'io.opentelemetry.aws-lambda-events-2.2',
        ready: (spans) => {
            const http = spans.find((s) => urlOf(s)?.includes('api_token='));
            expect(http, 'HTTP client span').toBeDefined();
            expect(urlOf(http)).toContain('api_token=****&page=1');
        },
    },
];

describe.concurrent('Masking in the extension (DASH0_MASK_IN_EXTENSION=true)', () => {
    for (const { family, handlerScopeName, ready } of cases) {
        it(`masks what the ${family} distro leaves unmasked`, async () => {
            const functionName = `${RESOURCE_PREFIX}masking-in-extension-${family}`;
            const invocationId = await invokeFunction(functionName, true, false);
            const { traceId } = await checkMainSpans({ invocationId, functionName, handlerScopeName });
            const { spans, logs } = await exported(functionName, invocationId, traceId, ready);
            assertNothingLeaked(spans, logs);
        }, TEST_TIMEOUT_MS);
    }
});
