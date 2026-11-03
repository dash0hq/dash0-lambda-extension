/*
 * Child-process runner for `opt/node/test/http-payload.test.ts`.
 *
 * The OpenTelemetry http instrumentation patches the `http` module through
 * require-in-the-middle, which jest's module loader bypasses. So this runs in plain Node:
 * it enables the compiled distro's `Dash0HttpInstrumentation`, then makes real
 * `http.request` calls against a local server and consumes each response the way
 * axios and node-fetch v2 do, piping it through their own zlib stream.
 *
 * Usage: node runner.mjs < scenarios.json   (a JSON array of scenarios on stdin)
 *
 *   encoding   optional `content-encoding` of the response
 *   wire       the response bytes, base64
 *   chunked    send the body in several network chunks instead of one
 *   contentType optional response content type (default application/json)
 *   setEncoding the client calls response.setEncoding() with this value, and reads text
 *
 * Prints one line `RESULT <JSON>` with, per scenario:
 *   { appBodyLength, responseBody, responseHeaders }
 * where appBodyLength is the size of what the application received after its own decoding.
 */

import fs from 'fs';
import path from 'path';
import { createRequire } from 'module';
import { fileURLToPath } from 'url';

const require = createRequire(import.meta.url);
const ROOT = path.join(path.dirname(fileURLToPath(import.meta.url)), '..', '..');
const scenarios = JSON.parse(fs.readFileSync(0, 'utf8'));

const { NodeTracerProvider } = require('@opentelemetry/sdk-trace-node');
const { InMemorySpanExporter, SimpleSpanProcessor } = require('@opentelemetry/sdk-trace-base');
const { default: Dash0HttpInstrumentation } =
  require(path.join(ROOT, 'distro/dist/src/instrumentations/https/HttpInstrumentation.js'));

const exporter = new InMemorySpanExporter();
const provider = new NodeTracerProvider({ spanProcessors: [new SimpleSpanProcessor(exporter)] });
provider.register();
const instrumentation = new Dash0HttpInstrumentation().getInstrumentation();
instrumentation.setTracerProvider(provider);
instrumentation.enable();

// Loaded after enable(), so require-in-the-middle patches it.
const http = require('http');
const zlib = require('zlib');

let current;
const server = http.createServer((request, response) => {
  request.resume();
  request.on('end', () => {
    const wire = Buffer.from(current.wire, 'base64');
    // The customer's response headers, behind Envoy and a CDN.
    response.writeHead(200, {
      'content-type': current.contentType || 'application/json',
      vary: 'origin, Accept-Encoding',
      ...(current.encoding ? { 'content-encoding': current.encoding } : {}),
      'access-control-allow-origin': '*',
      'x-envoy-upstream-service-time': '31',
      'cache-control': 'max-age=0, no-cache, no-store',
      pragma: 'no-cache',
      connection: 'keep-alive',
      ...(current.chunked ? {} : { 'content-length': wire.length }),
    });
    if (!current.chunked) return response.end(wire);
    response.write(wire.subarray(0, 7));
    setImmediate(() => {
      response.write(wire.subarray(7, 20));
      setImmediate(() => response.end(wire.subarray(20)));
    });
  });
});
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
const { port } = server.address();

const decoders = {
  gzip: zlib.createGunzip, 'x-gzip': zlib.createGunzip, deflate: zlib.createInflate,
  br: zlib.createBrotliDecompress,
};

function graphqlRequest() {
  return new Promise((resolve, reject) => {
    const request = http.request({
      host: '127.0.0.1', port, path: '/graphql', method: 'POST',
      headers: { 'content-type': 'application/json', 'accept-encoding': 'gzip, deflate, br' },
    }, response => {
      if (current.setEncoding) {
        response.setEncoding(current.setEncoding);
        let text = '';
        response.on('data', chunk => { text += chunk; });
        response.on('end', () => resolve(Buffer.from(text, current.setEncoding)));
        return;
      }
      const encoding = response.headers['content-encoding'];
      const decoder = Object.hasOwn(decoders, encoding) ? decoders[encoding] : undefined;
      const stream = decoder ? response.pipe(decoder()) : response;
      const chunks = [];
      stream.on('data', chunk => chunks.push(chunk));
      stream.on('end', () => resolve(Buffer.concat(chunks)));
      // A corrupt body is the application's problem; only the span matters here.
      stream.on('error', () => { response.resume(); resolve(Buffer.alloc(0)); });
    });
    request.on('error', reject);
    request.end(JSON.stringify({ query: '{ order(id: "order-42") { id status } }' }));
  });
}

const results = [];
for (const scenario of scenarios) {
  current = scenario;
  exporter.reset();
  const appBody = await graphqlRequest();
  // The client span ends on the response's 'end' event, after the application's handler.
  await new Promise(resolve => setTimeout(resolve, 10));
  const spans = exporter.getFinishedSpans().filter(span => span.kind === 2 /* CLIENT */);
  if (spans.length !== 1) throw new Error(`expected one client span, got ${spans.length}`);
  results.push({
    appBodyLength: appBody.length,
    responseBody: spans[0].attributes['http.response.body'],
    responseHeaders: spans[0].attributes['http.response.headers'],
  });
}

// Exit only once stdout is flushed: a pipe write can still be pending.
process.stdout.write('\nRESULT ' + JSON.stringify(results) + '\n', () => process.exit(0));
