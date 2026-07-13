import { describe, it } from 'vitest';
import { TEST_TIMEOUT_MS } from './config';
import { checkLogs, invokeFunction, LogToCheck, RESOURCE_PREFIX } from './utils';

// Both the event and the return value exceed the default DASH0_MAX_EVENT_PAYLOAD
// (20KB), so the extension must truncate the oversized string value while keeping
// the payload valid JSON with the short fields intact. The `password` field must
// come out masked, not truncated — masking runs before the size check.
describe.concurrent('Payload truncation', () => {
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
});
