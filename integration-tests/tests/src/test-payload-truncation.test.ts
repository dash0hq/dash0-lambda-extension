import { describe, it } from 'vitest';
import { TEST_TIMEOUT_MS } from './config';
import { checkLogs, invokeFunction, LogToCheck, RESOURCE_PREFIX } from './utils';

// Must match the extension's DASH0_MAX_EVENT_PAYLOAD default (1MB); the
// truncation-test function does not override it.
const MAX_PAYLOAD_BYTES = 1024 * 1024;

// Dash0 rejects log records whose string body exceeds 1 MiB, so the extension
// additionally shrinks a payload log body (wrapper and JSON escaping included)
// to fit. Mirrors MAX_LOG_BODY_BYTES and the escape-aware cut in the
// extension's log_mutations.rs.
const MAX_LOG_BODY_BYTES = 1024 * 1024;
const jsonEscapedLength = (c: string): number =>
    '"\\\b\f\n\r\t'.includes(c) ? 2 : c.charCodeAt(0) < 0x20 ? 6 : Buffer.byteLength(c);
const cutToEscapedBudget = (s: string, budget: number): string => {
    let used = 0;
    let end = 0;
    for (const c of s) {
        const cost = jsonEscapedLength(c);
        if (used + cost > budget) break;
        used += cost;
        end += c.length;
    }
    return s.slice(0, end);
};
// The message a non-JSON payload ends up with in its dash0_payload log record.
const expectedStringPayloadMessage = (payloadType: string, payload: string): string => {
    const truncated = payload.slice(0, MAX_PAYLOAD_BYTES);
    const wrapperBytes = Buffer.byteLength(
        JSON.stringify({ name: 'dash0_payload', type: payloadType, message: '' }),
    );
    return cutToEscapedBudget(truncated, MAX_LOG_BODY_BYTES - wrapperBytes);
};

describe.concurrent('Payload truncation', () => {
    // Both the event and the return value exceed the default limit, so the
    // extension must truncate the oversized string value while keeping the
    // payload valid JSON with the short fields intact. The `password` field
    // must come out masked, not truncated — masking runs before the size check.
    it('truncates oversized event and return value payloads after masking', async () => {
        const functionName = `${RESOURCE_PREFIX}payload-truncation`;
        const eventPayload = JSON.stringify({
            small: 'keep-me',
            password: 'event-secret',
            big: 'x'.repeat(1_100_000),
        });
        const invocationId = await invokeFunction(functionName, true, false, eventPayload);

        const logsToBeChecked: LogToCheck[] = [
            {
                message: JSON.stringify({
                    name: 'dash0_payload',
                    type: 'lambda_event',
                    message: { small: 'keep-me', password: '****', big: '[truncated]' },
                }),
                isJson: true,
                attributes: { 'dash0.faas.payload_type': 'lambda_event' },
            },
            {
                message: JSON.stringify({
                    name: 'dash0_payload',
                    type: 'lambda_return_value',
                    message: { statusCode: 200, small: 'keep-me', password: '****', big: '[truncated]' },
                }),
                isJson: true,
                attributes: { 'dash0.faas.payload_type': 'lambda_return_value' },
            },
        ];

        await checkLogs({
            invocationId,
            functionName,
            traceId: null,
            parentSpanId: null,
            success: true,
            logsToBeChecked,
        });
    }, TEST_TIMEOUT_MS);

    // Worst-case payloads for the truncation code in the extension, both of
    // which stalled the runtime proxy for tens of seconds (blowing the
    // function timeout) before truncation was made single-pass:
    // - Event: ~3.3MB of 100k short strings. Replacing every string still
    //   leaves ~1.4MB, over the 1MB limit, so the extension must detect
    //   infeasibility and fall back to a plain byte cut of the payload. The
    //   cut is embedded as an escaped string, which would push the log body
    //   over Dash0's 1 MiB body limit, so it is cut again to fit.
    // - Return value: ~4.7MB of 70k 64-byte strings where replacing all of
    //   them lands under the limit, so JSON-aware truncation has to replace
    //   ~69k of them — close to the most replacements it can ever perform
    //   within a 1MB limit. The result sits just under 1MB, so the log body
    //   wrapper pushes it over the body limit and one more string has to go.
    // The invocation completing at all (within the 10s function timeout) is
    // the performance assertion.
    it('handles worst-case payloads without stalling the invocation', async () => {
        const functionName = `${RESOURCE_PREFIX}payload-truncation`;
        const eventPayload = JSON.stringify(Array(100_000).fill('x'.repeat(30)));
        const invocationId = await invokeFunction(functionName, true, false, eventPayload);

        const logsToBeChecked: LogToCheck[] = [
            {
                // Infeasible for JSON-aware truncation: the logged event is a
                // plain byte cut, no longer valid JSON, embedded as a string
                // and cut again so the escaped body fits the log body limit.
                // Masking re-serializes compact JSON identically to
                // JSON.stringify here, so the cut is byte-exact.
                message: JSON.stringify({
                    name: 'dash0_payload',
                    type: 'lambda_event',
                    message: expectedStringPayloadMessage('lambda_event', eventPayload),
                }),
                isJson: true,
                attributes: { 'dash0.faas.payload_type': 'lambda_event' },
            },
            {
                // Feasible: items are replaced longest-first in document
                // order, so the first item is always the marker and the
                // result stays valid JSON.
                message: JSON.stringify({
                    name: 'dash0_payload',
                    type: 'lambda_return_value',
                    message: { statusCode: 200, items: ['[truncated]'] },
                }),
                isJson: true,
                attributes: { 'dash0.faas.payload_type': 'lambda_return_value' },
            },
        ];

        await checkLogs({
            invocationId,
            functionName,
            traceId: null,
            parentSpanId: null,
            success: true,
            logsToBeChecked,
        });
    }, TEST_TIMEOUT_MS);
});
