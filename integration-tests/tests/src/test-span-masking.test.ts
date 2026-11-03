import { describe, expect, it } from 'vitest';
import { TEST_TIMEOUT_MS } from './config';
import { checkMainSpans, getAttributesMap, invokeFunction, RESOURCE_PREFIX } from './utils';

describe.concurrent('SDK span attribute masking through the extension', () => {
    it.each([
        { mode: 'disabled', credential: 'sdk-credential', email: 'alice@example.com' },
        { mode: 'general', credential: '****', email: 'alice@example.com' },
        { mode: 'custom', credential: 'sdk-credential', email: '****' },
    ])('$mode rules apply to SDK-created attributes', async ({ mode, credential, email }) => {
        const functionName = `${RESOURCE_PREFIX}span-masking-${mode}`;
        const invocationId = await invokeFunction(functionName, true, false);
        const { handlerSpan } = await checkMainSpans({
            invocationId,
            functionName,
            handlerScopeName: '@opentelemetry/instrumentation-aws-lambda',
        });
        const attributes = getAttributesMap(handlerSpan.attributes);
        expect(attributes['customer.credential'].stringValue).toEqual(credential);
        expect(attributes['customer.email'].stringValue).toEqual(email);
        // The configured general rules replace defaults, and values are never searched.
        expect(attributes['customer.password'].stringValue).toEqual('default-rule-control');
        expect(attributes['customer.note'].stringValue).toEqual('{"password":"value-content-control"}');
    }, TEST_TIMEOUT_MS);
});
