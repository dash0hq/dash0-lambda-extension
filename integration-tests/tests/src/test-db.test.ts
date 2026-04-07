import { setTimeout as delay } from 'timers/promises';
import fetch from 'node-fetch';
import { describe, expect, it } from 'vitest';
import {
    checkOverheadSpan,
    checkLogs,
    checkMetrics,
    checkResourceAttributes,
    getAttributesMap,
    getRequestPayload,
    invokeFunction,
    RESOURCE_PREFIX,
} from './utils.js';
import {
    DASH0_ENDPOINT,
    DASH0_LAMBDA_TESTS_DATASET,
    DASH0_TOKEN,
    MAX_ATTEMPTS,
    RETRY_DELAY_MS,
    TEST_TIMEOUT_MS,
} from './config.js';

const nodeRuntimes = ['nodejs20-x', 'nodejs22-x', 'nodejs24-x'];
const pythonRuntimes = ['python3-12', 'python3-13', 'python3-14'];

interface DbSpanExpectation {
    scopeName: string;
    spanNames: string[];
    dbSystemName: string;
}

// Node.js expectations
const nodePostgresExpectation: DbSpanExpectation = {
    scopeName: '@opentelemetry/instrumentation-pg',
    spanNames: ['pg.connect', 'pg.query:CREATE testdb', 'pg.query:INSERT testdb', 'pg.query:SELECT testdb'],
    dbSystemName: 'postgresql',
};

const nodeMysqlExpectation: DbSpanExpectation = {
    scopeName: '@opentelemetry/instrumentation-mysql2',
    spanNames: ['CREATE', 'INSERT', 'SELECT'],
    dbSystemName: 'mysql',
};

// Python expectations
const pythonPostgresExpectation: DbSpanExpectation = {
    scopeName: 'opentelemetry.instrumentation.psycopg2',
    spanNames: ['CREATE', 'INSERT', 'fetchone', 'SELECT', 'fetchone'],
    dbSystemName: 'postgresql',
};

const pythonMysqlExpectation: DbSpanExpectation = {
    scopeName: 'opentelemetry.instrumentation.pymysql',
    spanNames: ['CREATE', 'INSERT', 'SELECT'],
    dbSystemName: 'mysql',
};

const HANDLER_SCOPE_NAMES = [
    '@opentelemetry/instrumentation-aws-lambda',
    'opentelemetry.instrumentation.aws_lambda',
];

const checkDbMainSpans = async ({
    invocationId,
    functionName,
}: {
    invocationId: string;
    functionName: string;
}): Promise<{ traceId: string; rootSpanId: string; handlerSpanId: string }> => {
    const now = Date.now();
    for (let attempt = 1; attempt <= MAX_ATTEMPTS; attempt++) {
        await delay(RETRY_DELAY_MS);
        console.log(`Attempt ${attempt} to fetch main spans for invocation ID ${invocationId}`);
        try {
            const spanResponse = await fetch(DASH0_ENDPOINT + 'spans', {
                method: 'POST',
                headers: {
                    accept: 'application/json',
                    authorization: `Bearer ${DASH0_TOKEN}`,
                    'content-type': 'application/json',
                },
                body: JSON.stringify(getRequestPayload(invocationId)),
            });

            const spanPayload = await spanResponse.json() as any;

            let handlerSpan: any = null;
            let handlerResource: any = null;
            const extensionSpans: any[] = [];
            let extensionResource: any = null;

            for (const rs of (spanPayload?.resourceSpans ?? [])) {
                for (const ss of (rs.scopeSpans ?? [])) {
                    if (HANDLER_SCOPE_NAMES.includes(ss.scope?.name)) {
                        expect(ss.spans.length).toEqual(1);
                        handlerSpan = ss.spans[0];
                        handlerResource = rs.resource;
                    }
                    if (ss.scope?.name === 'dash0.lambda-extension') {
                        extensionSpans.push(...(ss.spans ?? []));
                        extensionResource = rs.resource;
                    }
                }
            }

            expect(handlerSpan, 'Handler span not found').toBeDefined();

            const rootSpan = extensionSpans.find((s: any) => s.name === functionName);
            expect(rootSpan, `Root span not found for ${functionName}`).toBeDefined();

            expect(rootSpan.kind).toEqual(2); // SERVER
            expect(handlerSpan.kind).toEqual(1); // INTERNAL
            expect(handlerSpan.parentSpanId).toEqual(rootSpan.spanId);

            const traceId = rootSpan.traceId;
            expect(handlerSpan.traceId).toEqual(traceId);

            const handlerAttrs = getAttributesMap(handlerSpan.attributes);
            expect(handlerAttrs['faas.invocation_id'].stringValue).toEqual(invocationId);

            checkResourceAttributes(handlerResource.attributes, functionName);
            checkResourceAttributes(extensionResource.attributes, functionName);

            return {
                traceId,
                rootSpanId: rootSpan.spanId,
                handlerSpanId: handlerSpan.spanId,
            };
        } catch (error) {
            console.error(`Error fetching main spans on attempt ${attempt}:`, error);
            if (attempt === MAX_ATTEMPTS) {
                throw error;
            }
        }
    }
    throw new Error('checkDbMainSpans: exhausted all attempts');
};

const verifyDbInvocation = async (functionName: string, expectation: DbSpanExpectation) => {
    const invocationId = await invokeFunction(functionName, true, false);

    const { traceId, rootSpanId, handlerSpanId } = await checkDbMainSpans({
        invocationId,
        functionName,
    });

    await checkDbSpans({
        functionName,
        traceId,
        parentSpanId: handlerSpanId,
        expectation,
    });
};

const checkDbSpans = async ({
    functionName,
    traceId,
    parentSpanId,
    expectation,
}: {
    functionName: string;
    traceId: string;
    parentSpanId: string;
    expectation: DbSpanExpectation;
}) => {
    const now = Date.now();
    for (let attempt = 1; attempt <= MAX_ATTEMPTS; attempt++) {
        await delay(RETRY_DELAY_MS);
        console.log(`Attempt ${attempt} to fetch DB spans for ${functionName}`);
        try {
            const spanResponse = await fetch(DASH0_ENDPOINT + 'spans', {
                method: 'POST',
                headers: {
                    accept: 'application/json',
                    authorization: `Bearer ${DASH0_TOKEN}`,
                    'content-type': 'application/json',
                },
                body: JSON.stringify({
                    filter: [
                        {
                            operator: 'is',
                            key: 'service.name',
                            value: functionName,
                        },
                        {
                            operator: 'is',
                            key: 'db.system.name',
                            value: expectation.dbSystemName,
                        },
                    ],
                    timeRange: {
                        from: new Date(now - 5 * 60_000).toISOString(),
                        to: new Date(now + 5 * 60_000).toISOString(),
                    },
                    sampling: { mode: 'adaptive' },
                    dataset: DASH0_LAMBDA_TESTS_DATASET,
                }),
            });

            const spanPayload = await spanResponse.json() as any;
            expect(spanPayload?.resourceSpans.length).toBeGreaterThanOrEqual(1);

            const dbSpans: any[] = [];
            for (const rs of spanPayload.resourceSpans) {
                for (const ss of rs.scopeSpans) {
                    if (ss.scope?.name === expectation.scopeName) {
                        for (const span of ss.spans) {
                            if (span.traceId === traceId && span.parentSpanId === parentSpanId) {
                                dbSpans.push(span);
                            }
                        }
                    }
                }
            }

            expect(dbSpans.length, `Expected ${expectation.spanNames.length} DB spans, got ${dbSpans.length}`).toBeGreaterThanOrEqual(expectation.spanNames.length);

            const remainingSpans = [...dbSpans];
            for (const expectedName of expectation.spanNames) {
                const idx = remainingSpans.findIndex((s: any) => s.name === expectedName);
                expect(idx, `DB span with name "${expectedName}" not found`).toBeGreaterThanOrEqual(0);

                const matchingSpan = remainingSpans.splice(idx, 1)[0];
                const attrs = getAttributesMap(matchingSpan.attributes);
                expect(attrs['db.system.name']?.stringValue).toEqual(expectation.dbSystemName);
                expect(attrs['db.namespace']?.stringValue).toEqual('testdb');
                expect(matchingSpan.kind).toEqual(3); // CLIENT
            }

            return;
        } catch (error) {
            console.error(`Error fetching DB spans on attempt ${attempt}:`, error);
            if (attempt === MAX_ATTEMPTS) {
                throw error;
            }
        }
    }
};

describe.concurrent('DB tracing', () => {
    for (const runtime of nodeRuntimes) {
        const runtimeName = runtime.replace(/\./g, '-');

        const postgresFunctionName = `${RESOURCE_PREFIX}db-testing-rds-postgres-${runtimeName}`;
        it(
            `traces PostgreSQL queries for ${postgresFunctionName}`,
            async () => {
                console.log(`Starting DB test for ${postgresFunctionName}`, new Date().toISOString());
                await verifyDbInvocation(postgresFunctionName, nodePostgresExpectation);
            },
            TEST_TIMEOUT_MS,
        );

        const mysqlFunctionName = `${RESOURCE_PREFIX}db-testing-rds-mysql-${runtimeName}`;
        it(
            `traces MySQL queries for ${mysqlFunctionName}`,
            async () => {
                console.log(`Starting DB test for ${mysqlFunctionName}`, new Date().toISOString());
                await verifyDbInvocation(mysqlFunctionName, nodeMysqlExpectation);
            },
            TEST_TIMEOUT_MS,
        );
    }

    for (const runtime of pythonRuntimes) {
        const runtimeName = runtime.replace(/\./g, '-');

        const postgresFunctionName = `${RESOURCE_PREFIX}db-testing-rds-postgres-${runtimeName}`;
        it(
            `traces PostgreSQL queries for ${postgresFunctionName}`,
            async () => {
                console.log(`Starting DB test for ${postgresFunctionName}`, new Date().toISOString());
                await verifyDbInvocation(postgresFunctionName, pythonPostgresExpectation);
            },
            TEST_TIMEOUT_MS,
        );

        const mysqlFunctionName = `${RESOURCE_PREFIX}db-testing-rds-mysql-${runtimeName}`;
        it(
            `traces MySQL queries for ${mysqlFunctionName}`,
            async () => {
                console.log(`Starting DB test for ${mysqlFunctionName}`, new Date().toISOString());
                await verifyDbInvocation(mysqlFunctionName, pythonMysqlExpectation);
            },
            TEST_TIMEOUT_MS,
        );
    }
});
