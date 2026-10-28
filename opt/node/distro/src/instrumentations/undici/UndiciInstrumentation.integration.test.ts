import { createServer, Server } from 'http';
import type { AddressInfo } from 'net';
import { gzipSync } from 'zlib';
import { trace } from '@opentelemetry/api';
import {
  InMemorySpanExporter,
  SimpleSpanProcessor,
} from '@opentelemetry/sdk-trace-base';
import type { ReadableSpan } from '@opentelemetry/sdk-trace-base';
import { NodeTracerProvider } from '@opentelemetry/sdk-trace-node';
import Dash0UndiciInstrumentation from './UndiciInstrumentation';

// Real fetch traffic and exported spans, without mocking the instrumentation hooks.
// The server supplies GraphQL wire-format fixtures, not a GraphQL execution engine.
const cases = [
  {
    name: 'GraphQL',
    path: '/graphql',
    request: {
      operationName: 'FindProduct',
      query: 'query FindProduct($id: ID!) { product(id: $id) { id name } }',
      variables: { id: 'product-42' },
    },
    response: { data: { product: { id: 'product-42', name: 'Coffee' } } },
  },
  {
    name: 'ordinary JSON control',
    path: '/products',
    request: { id: 'product-17' },
    response: { id: 'product-17', name: 'Tea' },
  },
].flatMap(fixture => [false, true].flatMap(gzip =>
  ['application/json', 'application/graphql-response+json'].map(mediaType => ({
    ...fixture,
    name: `${fixture.name}, ${mediaType}, ${gzip ? 'gzip' : 'plain'}`,
    path: `${fixture.path}/${mediaType.split('/')[1]}/${gzip}`,
    gzip,
    mediaType,
    response: { ...fixture.response, password: 'response-canary', sessionToken: 'token-canary' },
  }))));

const safetyCases: {
  name: string;
  text: string;
  capture: string | undefined;
  wire?: Buffer;
  gzip?: boolean;
  members?: boolean;
  mediaType?: string;
  rules?: string;
  limit?: string;
}[] = [
  { name: 'invalid JSON', text: '{"password":"do-not-export"', capture: undefined },
  { name: 'invalid UTF-8', text: '{"n":"�"}', wire: Buffer.from([123, 34, 110, 34, 58, 34, 255, 34, 125]), capture: undefined },
  { name: 'export limit', text: JSON.stringify({ padding: 'x'.repeat(5000) }), capture: undefined },
  { name: 'UTF-8 byte export limit', text: JSON.stringify({ note: 'é'.repeat(2500) }), capture: undefined },
  { name: 'encoded limit', text: JSON.stringify({ padding: 'x'.repeat(32768) }), capture: undefined },
  { name: 'gzip bomb', text: JSON.stringify({ padding: 'x'.repeat(1024 * 1024) }), gzip: true, capture: undefined },
  { name: 'concatenated gzip bomb', text: JSON.stringify({ padding: 'x'.repeat(65536) }), gzip: true, members: true, capture: undefined },
  { name: 'deep JSON', text: '['.repeat(40) + '1' + ']'.repeat(40), capture: undefined },
  { name: 'many keys', text: JSON.stringify(Array.from({ length: 2050 }, () => 1)), limit: '65536', capture: undefined },
  { name: 'long key', text: JSON.stringify({ ['a'.repeat(129)]: 'value' }), capture: undefined },
  { name: 'non-JSON', text: 'do-not-export', mediaType: 'text/plain', capture: undefined },
  { name: 'all', text: '{"password":"hidden","orderId":42}', rules: 'all', capture: '"****"' },
  { name: 'override', text: '{"email":"hidden","orderId":42}', rules: '["^email$"]', capture: '{"email":"****","orderId":42}' },
  { name: 'empty rules', text: '{"password":"explicitly-unmasked","orderId":42}', rules: '[]', capture: '{"password":"explicitly-unmasked","orderId":42}' },
  { name: 'own prototype key', text: '{"__proto__":{"password":"hidden"},"orderId":42}', capture: '{"__proto__":{"password":"****"},"orderId":42}' },
  { name: 'mask own prototype key', text: '{"__proto__":{"password":"hidden"},"orderId":42}', rules: '["^__proto__$"]', capture: '{"__proto__":"****","orderId":42}' },
  { name: 'matched null', text: '{"password":null,"orderId":42}', capture: '{"password":"****","orderId":42}' },
  { name: 'all with null', text: 'null', rules: 'all', capture: '"****"' },
  ...[false, true].map(gzip => ({
    name: `pre-mask omission even with all (${gzip ? 'gzip' : 'identity'})`,
    text: JSON.stringify({ password: 'x'.repeat(5000) }), rules: 'all', gzip, capture: undefined,
  })),
  { name: 'unsafe regex', text: '{"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!":42}', rules: '["(a+)+$"]', capture: undefined },
  { name: 'invalid rules', text: '{"password":"hidden"}', rules: '[', capture: undefined },
  ...[4095, 4096, 4097].map(size => {
    const text = JSON.stringify({ n: 'x'.repeat(size - 8) });
    return { name: `export boundary ${size}`, text, capture: size <= 4096 ? text : undefined };
  }),
  ...[32767, 32768, 32769].map(size => {
    const text = JSON.stringify({ n: 'x'.repeat(size - 8) });
    return { name: `encoded boundary ${size}`, text, limit: '65536', capture: size <= 32768 ? text : undefined };
  }),
  ...[65535, 65536, 65537].map(size => {
    const text = JSON.stringify({ n: 'x'.repeat(size - 8) });
    return { name: `decoded boundary ${size}`, text, gzip: true, limit: '65536', capture: size <= 65536 ? text : undefined };
  }),
];

describe('Undici response payload reproduction', () => {
  const originalFetch = globalThis.fetch;
  const exporter = new InMemorySpanExporter();
  const provider = new NodeTracerProvider({
    spanProcessors: [new SimpleSpanProcessor(exporter)],
  });
  let instrumentation: ReturnType<Dash0UndiciInstrumentation['getInstrumentation']>;
  let server: Server;
  let origin: string;
  const receivedBodies: Record<string, unknown> = {};

  beforeAll(async () => {
    provider.register();
    instrumentation = new Dash0UndiciInstrumentation().getInstrumentation();
    instrumentation.setTracerProvider(provider);
    instrumentation.enable();

    server = createServer(async (request, response) => {
      if (request.url?.startsWith('/safety/')) {
        const fixture = safetyCases[Number(request.url.split('/')[2])];
        const body = fixture.wire || Buffer.from(fixture.text);
        response.writeHead(200, {
          'content-type': fixture.mediaType || 'application/json',
          ...(fixture.gzip ? { 'content-encoding': 'gzip' } : {}),
        });
        response.end(fixture.members
          ? Buffer.concat([gzipSync(body.subarray(0, -2)), gzipSync(body.subarray(-2))])
          : fixture.gzip ? gzipSync(body) : body);
        return;
      }
      const chunks: Buffer[] = [];
      for await (const chunk of request) chunks.push(Buffer.from(chunk));
      receivedBodies[request.url!] = JSON.parse(Buffer.concat(chunks).toString());
      const fixture = cases.find(testCase => testCase.path === request.url)!;
      const body = Buffer.from(JSON.stringify(fixture.response));
      const encoded = fixture.gzip ? gzipSync(body) : body;
      response.writeHead(200, {
        'content-type': fixture.mediaType,
        'content-length': encoded.length,
        ...(fixture.gzip ? { 'content-encoding': 'gzip' } : {}),
      });
      // Split chunks to exercise capture without taking ownership of the Fetch body.
      response.write(encoded.subarray(0, 7));
      setImmediate(() => response.end(encoded.subarray(7)));
    });
    await new Promise<void>(resolve => server.listen(0, '127.0.0.1', resolve));
    origin = `http://127.0.0.1:${(server.address() as AddressInfo).port}`;
  });

  afterAll(async () => {
    instrumentation?.disable();
    globalThis.fetch = originalFetch;
    if (server) {
      await new Promise<void>((resolve, reject) => {
        server.close(error => error ? reject(error) : resolve());
        server.closeAllConnections();
      });
    }
    await provider.shutdown();
    trace.disable();
  });

  describe.each(cases)('$name', fixture => {
    let span: ReadableSpan;

    beforeAll(async () => {
      exporter.reset();
      const response = await fetch(`${origin}${fixture.path}`, {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify(fixture.request),
      });

      // Prove that a body arrived and the application consumed it before export.
      expect(response.status).toBe(200);
      expect(await response.json()).toEqual(fixture.response);
      expect(receivedBodies[fixture.path]).toEqual(fixture.request);
      await provider.forceFlush();

      const spans = exporter.getFinishedSpans().filter(
        candidate => candidate.instrumentationScope.name === '@opentelemetry/instrumentation-undici'
      );
      expect(spans).toHaveLength(1);
      span = spans[0];
    });

    test('exports the request body and response headers', () => {
      expect(JSON.parse(span.attributes['http.request.body'] as string)).toEqual(fixture.request);
      expect(span.attributes['http.response.status_code']).toBe(200);
      expect(JSON.parse(span.attributes['http.response.headers'] as string)).toMatchObject({
        'content-type': fixture.mediaType,
      });
    });

    test('exports the complete masked response before the span ends', () => {
      expect(JSON.parse(span.attributes['http.response.body'] as string)).toEqual({
        ...fixture.response, password: '****', sessionToken: '****',
      });
    });
  });

  test.each(safetyCases)('preserves application bytes and fails closed: $name', async fixture => {
    const key = 'LUMIGO_SECRET_MASKING_REGEX_HTTP_RESPONSE_BODIES';
    const previous = process.env[key];
    const previousLimit = process.env.OTEL_SPAN_ATTRIBUTE_VALUE_LENGTH_LIMIT;
    if (fixture.limit) process.env.OTEL_SPAN_ATTRIBUTE_VALUE_LENGTH_LIMIT = fixture.limit;
    if (fixture.rules) process.env[key] = fixture.rules;
    exporter.reset();
    try {
      const response = await fetch(`${origin}/safety/${safetyCases.indexOf(fixture)}`);
      expect(await response.text()).toBe(fixture.text);
      await provider.forceFlush();
      const spans = exporter.getFinishedSpans().filter(
        candidate => candidate.instrumentationScope.name === '@opentelemetry/instrumentation-undici'
      );
      expect(spans).toHaveLength(1);
      expect(spans[0].attributes['http.response.body']).toBe(fixture.capture);
    } finally {
      if (previous === undefined) delete process.env[key];
      else process.env[key] = previous;
      if (previousLimit === undefined) delete process.env.OTEL_SPAN_ATTRIBUTE_VALUE_LENGTH_LIMIT;
      else process.env.OTEL_SPAN_ATTRIBUTE_VALUE_LENGTH_LIMIT = previousLimit;
    }
  });
});
