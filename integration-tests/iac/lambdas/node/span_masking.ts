import { trace } from '@opentelemetry/api';

// Use the layer's SDK and exporter, not a provider or flush installed by the fixture.
export async function handler() {
    const span = trace.getActiveSpan();
    if (!span) {
        throw new Error('The layer did not create an active handler span');
    }
    span.setAttributes({
        'customer.credential': 'sdk-credential',
        'customer.email': 'alice@example.com',
        'customer.password': 'default-rule-control',
        'customer.note': '{"password":"value-content-control"}',
    });
    return { statusCode: 200 };
}
