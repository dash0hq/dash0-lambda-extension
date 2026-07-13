import { describe, it } from 'vitest';
import { TEST_TIMEOUT_MS } from './config';
import { checkLogs, invokeFunction, LogToCheck, RESOURCE_PREFIX } from './utils';

// Must match the extension's DASH0_MAX_EVENT_PAYLOAD default (4KB); the
// truncation-test function does not override it.
const MAX_PAYLOAD_BYTES = 4 * 1024;

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
            big: 'x'.repeat(25_000),
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
    // - Event: ~3.3MB of 100k short strings. Replacing every string can't
    //   reach the 4KB limit, so the extension must detect infeasibility and
    //   fall back to a plain byte cut of the payload.
    // - Return value: ~5MB of 280 long strings where replacing all of them
    //   lands just under the limit — the maximum number of replacements
    //   JSON-aware truncation can ever perform.
    // The invocation completing at all (within the 10s function timeout) is
    // the performance assertion.
    it('handles worst-case payloads without stalling the invocation', async () => {
        const functionName = `${RESOURCE_PREFIX}payload-truncation`;
        const eventPayload = JSON.stringify(Array(100_000).fill('x'.repeat(30)));
        const invocationId = await invokeFunction(functionName, true, false, eventPayload);

        const logsToBeChecked: LogToCheck[] = [
            {
                // Infeasible for JSON-aware truncation: the logged event is a
                // plain byte cut, no longer valid JSON, embedded as a string.
                // Masking re-serializes compact JSON identically to
                // JSON.stringify here, so the cut is byte-exact.
                message: JSON.stringify({
                    name: 'dash0_payload',
                    type: 'lambda_event',
                    message: eventPayload.slice(0, MAX_PAYLOAD_BYTES),
                }),
                isJson: true,
                attributes: { 'dash0.faas.payload_type': 'lambda_event' },
            },
            {
                // Feasible: every item is replaced by the marker and the
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
