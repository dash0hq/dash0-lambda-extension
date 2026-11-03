/*
 * Pins the Kinesis trace context injection that `scripts/patches/` adds to
 * `@opentelemetry/instrumentation-aws-sdk`. Upstream does not have it, so a tag bump that
 * drops or breaks the patch would silently stop consumers from linking to producers.
 */
import { trace } from '@opentelemetry/api';
import { W3CTraceContextPropagator } from '@opentelemetry/core';
import { NodeTracerProvider } from '@opentelemetry/sdk-trace-node';
// The service extension is not exported from the package root.
import { KinesisServiceExtension } from '@opentelemetry/instrumentation-aws-sdk/build/src/services/kinesis';

const KINESIS_RECORD_LIMIT = 1_048_576;

describe('Kinesis trace context injection (vendored aws-sdk patch)', () => {
  const extension = new KinesisServiceExtension();
  const tracer = trace.getTracer('kinesis-context-injection-test');

  beforeAll(() => {
    new NodeTracerProvider().register({ propagator: new W3CTraceContextPropagator() });
  });

  afterAll(() => {
    trace.disable();
  });

  /** Runs the post-span hook inside an active span, as the instrumentation does. */
  const putWithinSpan = (commandName: string, commandInput: Record<string, any>) =>
    tracer.startActiveSpan('producer', (span) => {
      extension.requestPostSpanHook({ commandName, commandInput } as any);
      span.end();
      const { traceId, spanId } = span.spanContext();
      return `00-${traceId}-${spanId}-01`;
    });

  test('PutRecord: adds traceparent to JSON Data and keeps it a byte array', () => {
    const input = {
      StreamName: 'orders',
      Data: new TextEncoder().encode(JSON.stringify({ orderId: 7 })),
    };

    const traceparent = putWithinSpan('PutRecord', input);

    expect(input.Data).toBeInstanceOf(Uint8Array);
    expect(JSON.parse(new TextDecoder().decode(input.Data))).toEqual({ orderId: 7, traceparent });
  });

  test('PutRecords: adds traceparent to every record, including string Data', () => {
    const input = {
      StreamName: 'orders',
      Records: [{ Data: JSON.stringify({ n: 1 }) }, { Data: new TextEncoder().encode('{"n":2}') }],
    };

    const traceparent = putWithinSpan('PutRecords', input);

    expect(JSON.parse(input.Records[0].Data as string)).toEqual({ n: 1, traceparent });
    expect(JSON.parse(new TextDecoder().decode(input.Records[1].Data as Uint8Array))).toEqual({
      n: 2,
      traceparent,
    });
  });

  test('leaves non-JSON Data unchanged', () => {
    const input = { StreamName: 'orders', Data: 'not json' };

    putWithinSpan('PutRecord', input);

    expect(input.Data).toBe('not json');
  });

  test('leaves Data unchanged when the injected record would exceed the 1 MB limit', () => {
    // Fits on its own, but not with a ~70-byte traceparent added.
    const payload = JSON.stringify({ blob: 'x'.repeat(KINESIS_RECORD_LIMIT - 20) });
    const input = { StreamName: 'orders', Data: payload };

    putWithinSpan('PutRecord', input);

    expect(input.Data).toBe(payload);
  });

  test('does not touch other commands', () => {
    const data = JSON.stringify({ n: 1 });
    const input = { StreamName: 'orders', Data: data };

    putWithinSpan('DescribeStream', input);

    expect(input.Data).toBe(data);
  });
});
