import type { RequestOptions, Server } from 'http';

import { SpanKind } from '@opentelemetry/api';
import {
  BasicTracerProvider,
  InMemorySpanExporter,
  SimpleSpanProcessor,
} from '@opentelemetry/sdk-trace-base';

import Dash0HttpInstrumentation from './HttpInstrumentation';

describe('Dash0HttpInstrumentation', () => {
  let dash0HttpInstrumentation = new Dash0HttpInstrumentation();

  test('getInstrumentedModule should return "http"', () => {
    expect(dash0HttpInstrumentation.getInstrumentedModule()).toEqual('http');
  });

  /*
   * instrumentation-http reads `options.host` and calls .indexOf()/.match() on it without
   * checking that it is a string. Node resolves the target from `options.hostname` first and
   * ignores `options.host` entirely whenever `hostname` is set, so a non-string `host` is
   * inert for Node but blows up inside the instrumentation -- taking the caller's request,
   * and with it the whole Lambda invocation, down with it. See SUP-1457.
   */
  describe('outgoing requests with a non-string options.host', () => {
    const exporter = new InMemorySpanExporter();
    let server: Server;
    let http: typeof import('http');
    let port: number;

    beforeAll(async () => {
      const instrumentation = new Dash0HttpInstrumentation().getInstrumentation();
      instrumentation.setTracerProvider(
        new BasicTracerProvider({ spanProcessors: [new SimpleSpanProcessor(exporter)] })
      );

      /*
       * require-in-the-middle cannot hook jest's module registry, so instead of enable()
       * we apply the instrumentation's own patch to a copy of the http module. This runs
       * the real upstream wrapper over the real Dash0 config.
       */
      // eslint-disable-next-line @typescript-eslint/no-require-imports
      const httpModule = require('http') as typeof import('http');
      const definition = instrumentation
        .getModuleDefinitions()
        .find(({ name }) => name === 'http');
      http = definition.patch({ ...httpModule }) ?? httpModule;

      server = httpModule.createServer((_req, res) => res.end('ok'));
      await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', resolve));
      port = (server.address() as { port: number }).port;
    });

    afterAll(async () => {
      await new Promise((resolve) => server.close(resolve));
    });

    beforeEach(() => exporter.reset());

    const request = (options: RequestOptions) =>
      new Promise<void>((resolve, reject) => {
        const req = http.request(options, (res) => {
          res.resume();
          res.on('end', resolve);
        });
        req.on('error', reject);
        req.end();
      });

    const clientSpanAttributes = () =>
      exporter.getFinishedSpans().find((span) => span.kind === SpanKind.CLIENT)?.attributes;

    const optionsWithHost = (host: unknown): RequestOptions =>
      ({
        hostname: '127.0.0.1',
        port: String(port),
        path: '/some/path',
        method: 'GET',
        host,
      }) as RequestOptions;

    test.each([
      ['a URL object', () => new URL(`http://127.0.0.1:${port}/some/path`)],
      ['a plain object', () => ({ endpoint: 'http://127.0.0.1/some/path' })],
      ['a number', () => port],
    ])('completes the request and traces it when host is %s', async (_label, buildHost) => {
      await expect(request(optionsWithHost(buildHost()))).resolves.toBeUndefined();

      expect(clientSpanAttributes()).toMatchObject({
        'http.url': `http://127.0.0.1:${port}/some/path`,
        'net.peer.name': '127.0.0.1',
        'net.peer.port': port,
      });
    });

    test('leaves Node to reject a non-string host that has no usable hostname', async () => {
      // Node itself rejects these options, instrumented or not. The instrumentation must not
      // replace Node's error with one of its own.
      await expect(
        request({
          host: new URL(`http://127.0.0.1:${port}/some/path`),
          port: String(port),
          path: '/some/path',
        } as unknown as RequestOptions)
      ).rejects.toMatchObject({ code: 'ERR_INVALID_ARG_TYPE' });
    });

    test('still traces requests that use a string host', async () => {
      await expect(
        request({ host: '127.0.0.1', port: String(port), path: '/some/path' })
      ).resolves.toBeUndefined();

      expect(clientSpanAttributes()).toMatchObject({
        'http.url': `http://127.0.0.1:${port}/some/path`,
        'net.peer.name': '127.0.0.1',
      });
    });
  });
});
