// Local, crash-isolated validation. Callback fault tests are NOT a real HTTP soak.
import assert from 'node:assert/strict';
import { fork } from 'node:child_process';
import { createHash } from 'node:crypto';
import { createServer } from 'node:http';
import { createRequire } from 'node:module';
import { performance } from 'node:perf_hooks';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { gzipSync } from 'node:zlib';

const script = fileURLToPath(import.meta.url);
const root = path.resolve(path.dirname(script), '../..');
const require = createRequire(path.join(root, 'opt/node/package.json'));
const modulePath = path.join(root, 'opt/node/distro/dist/src/instrumentations/undici/responseBody.js');
const fixture = () => JSON.stringify({ data: { product: {
  id: 42, name: 'Coffee', password: ['http', 'canary'].join('-'),
} } });

if (process.argv[2] === 'callbacks') {
  const { captureResponseBody } = require(modulePath);
  const fault = process.argv[3];
  let seed = 823;
  const random = () => (seed = (Math.imul(seed, 1664525) + 1013904223) >>> 0);
  const headers = { 'content-type': 'application/graphql-response+json' };
  const sentinel = new Error('original-error');
  let dataCalls = 0, completeCalls = 0, errorCalls = 0;
  class Request {
    onData(chunk, token) { assert.equal(this.token, token); dataCalls++; return false; }
    onComplete(token) { assert.equal(this.token, token); completeCalls++; return token; }
    onError(error) { assert.equal(error, sentinel); errorCalls++; throw error; }
  }
  let attribute;
  const span = { isRecording: () => true, setAttribute(key, value) {
    assert.equal(key, 'http.response.body'); attribute = value;
    if (fault === 'span') throw new Error('capture-error');
  } };
  const install = () => { const r = new Request(); r.token = {}; captureResponseBody(r, span, headers); return r; };
  if (fault === 'capacity') {
    const requests = Array.from({ length: 17 }, install);
    assert.ok(Object.hasOwn(requests[15], 'onData'));
    assert.ok(!Object.hasOwn(requests[16], 'onData'));
    for (const r of requests) assert.throws(() => r.onError(sentinel), e => e === sentinel);
    const r = install(); assert.ok(Object.hasOwn(r, 'onData'));
    assert.throws(() => r.onError(sentinel), e => e === sentinel);
  } else if (fault === 'original') {
    for (const name of ['onData', 'onComplete', 'onError']) {
      const prototype = Object.create(Request.prototype);
      prototype[name] = function (...args) { assert.equal(this.token, args[1]); throw sentinel; };
      const request = Object.create(prototype);
      request.token = {};
      captureResponseBody(request, span, headers);
      assert.throws(() => request[name](Buffer.from('{}'), request.token), error => error === sentinel);
      assert.ok(!Object.hasOwn(request, 'onData'));
    }
    process.send({ fault, originalErrorsPreserved: 3 });
  } else if (fault === 'borrowed') {
    const request = install(), other = new Request(); other.token = {};
    assert.equal(request.onData.call(other, Buffer.from('{}'), other.token), false);
    assert.equal(request.onComplete.call(other, other.token), other.token);
    assert.throws(() => request.onError.call(other, sentinel), error => error === sentinel);
    assert.equal(attribute, undefined);
    assert.throws(() => request.onError(sentinel), error => error === sentinel);
    process.send({ fault, receiverAndArgumentsPreserved: true });
  } else if (fault === 'frozen' || fault === 'replaced') {
    const request = install(), wrapped = request.onData;
    request.onData(Buffer.from('{"password":"canary"}'), request.token);
    const replacement = () => 99;
    if (fault === 'frozen') Object.freeze(request);
    else request.onData = replacement;
    assert.equal(request.onComplete(request.token), request.token);
    assert.equal(attribute, '{"password":"****"}');
    if (fault === 'replaced') assert.equal(request.onData, replacement);
    attribute = undefined;
    assert.equal(wrapped.call(request, Buffer.from('{}'), request.token), false);
    assert.equal(attribute, undefined);
    const next = install(); assert.ok(Object.hasOwn(next, 'onData'));
    assert.throws(() => next.onError(sentinel), error => error === sentinel);
    process.send({ fault, cleanupAndCapacityRecovered: true });
  } else {
    const originalAlloc = Buffer.alloc;
    const originalAllocUnsafe = Buffer.allocUnsafe;
    const originalParse = JSON.parse;
    const originalDefine = Object.defineProperties;
    const originalCopy = Buffer.prototype.copy;
    if (fault === 'alloc') Buffer.alloc = Buffer.allocUnsafe = () => { throw new Error('capture-error'); };
    if (fault === 'parse') JSON.parse = () => { throw new Error('capture-error'); };
    if (fault === 'copy') Buffer.prototype.copy = () => { throw new Error('capture-error'); };
    if (fault === 'copy-short') Buffer.prototype.copy = () => 0;
    if (fault === 'install') Object.defineProperties = (target, descriptors) => {
      Object.defineProperty(target, 'onData', descriptors.onData);
      throw new Error('capture-error');
    };
    const batches = [];
    const iterations = fault === 'random' ? Number(process.env.CALLBACK_ITERATIONS || 1000000) : 100;
    const times = [];
    for (let i = 0; i < iterations; i++) {
      attribute = undefined;
      const r = install();
      const body = Buffer.from('{"password":"canary","id":42}');
      const start = performance.now();
      let offset = 0;
      while (offset < body.length) {
        const end = Math.min(body.length, offset + 1 + random() % 13);
        const chunk = body.subarray(offset, end);
        const before = Buffer.from(chunk);
        assert.equal(r.onData(chunk, r.token), false);
        assert.deepEqual(chunk, before);
        offset = end;
      }
      if (fault === 'random' && random() % 7 === 0) {
        assert.throws(() => r.onError(sentinel), e => e === sentinel);
        assert.equal(attribute, undefined);
      } else {
        assert.equal(r.onComplete(r.token), r.token);
        if (fault === 'random') assert.equal(attribute, '{"password":"****","id":42}');
      }
      if (['alloc', 'copy', 'copy-short', 'parse', 'install'].includes(fault)) assert.equal(attribute, undefined);
      if (fault === 'span') assert.equal(attribute, '{"password":"****","id":42}');
      if (times.length < 10000) times.push(performance.now() - start);
      if (i % Math.max(1, Math.floor(iterations / 10)) === 0) {
        global.gc(); batches.push(process.memoryUsage());
      }
    }
    Buffer.alloc = originalAlloc; Buffer.allocUnsafe = originalAllocUnsafe; JSON.parse = originalParse;
    Object.defineProperties = originalDefine; Buffer.prototype.copy = originalCopy;
    const r = install(); assert.ok(Object.hasOwn(r, 'onData'));
    assert.throws(() => r.onError(sentinel), e => e === sentinel);
    times.sort((a, b) => a - b);
    process.send({ fault, iterations, dataCalls, completeCalls, errorCalls, batches,
      callbackCycleP99Ms: times[Math.ceil(times.length * .99) - 1] });
  }
  if (fault === 'capacity') process.send({ fault, errorCalls });
} else if (process.argv[2] === 'http') {
  const config = JSON.parse(process.argv[3]);
  config.body = fixture();
  let provider, meter, instrumentation;
  if (config.bundle) {
    await import(pathToFileURL(config.bundle).href);
    const api = require('@opentelemetry/api');
    provider = api.trace.getTracerProvider().getDelegate();
    meter = api.metrics.getMeterProvider();
  } else if (config.instrumented) {
    const { NodeTracerProvider } = require('@opentelemetry/sdk-trace-node');
    provider = new NodeTracerProvider({ spanProcessors: [{ onStart() {}, onEnd() {},
      async shutdown() {}, async forceFlush() {} }] });
    provider.register();
    const Cls = require(config.module || path.join(root, 'opt/node/distro/dist/src/instrumentations/undici/UndiciInstrumentation.js')).default;
    instrumentation = new Cls().getInstrumentation();
    instrumentation.setTracerProvider(provider);
  }
  const hash = createHash('sha256');
  const batches = [];
  const count = config.count || 100;
  let next = 0;
  const outcomeArrayLengths = [];
  for (let batch = 0; batch < 10; batch++) {
    next = 0;
    const outcomes = [];
    await Promise.all(Array.from({ length: 50 }, async () => {
      while (next++ < count) {
        const id = next + batch * count;
        // Sparse indexing reproduced the old harness's growing result-array cost.
        // Default to a fixed-size batch, without changing request IDs or outcomes.
        const index = process.env.SOAK_INDEX === 'sparse' ? id : next - 1;
        const mode = id % 5;
        try {
          const response = await fetch(`http://127.0.0.1:${config.port}/${mode}`);
          if (mode === 3) {
            await response.body.cancel();
            outcomes[index] = 'cancelled';
          } else {
            const text = await response.text();
            if (mode !== 4) assert.equal(text, config.body);
            outcomes[index] = `${mode}:${response.status}:${text}`;
          }
        } catch (error) {
          assert.equal(mode, 4); // intentionally corrupt gzip
          assert.equal(error.name, 'TypeError');
          outcomes[index] = `${mode}:${error.name}:${error.message}`;
        }
      }
    }));
    hash.update(JSON.stringify(outcomes));
    outcomeArrayLengths.push(outcomes.length);
    global.gc(); batches.push(process.memoryUsage());
  }
  await provider?.forceFlush().catch(() => {});
  instrumentation?.disable();
  await provider?.shutdown().catch(() => {});
  await meter?.shutdown().catch(() => {});
  const idleMemory = [];
  if (process.env.POST_SOAK_IDLE_MS) {
    for (let i = 0; i < 10; i++) {
      await new Promise(resolve => setTimeout(resolve, Number(process.env.POST_SOAK_IDLE_MS) / 10));
      global.gc(); idleMemory.push(process.memoryUsage());
    }
  }
  delete config.body;
  process.send({ config, batches, idleMemory, outcomeArrayLengths, digest: hash.digest('hex') });
} else {
  const run = (mode, config, env = {}) => new Promise((resolve, reject) => {
    const child = fork(script, [mode, typeof config === 'string' ? config : JSON.stringify(config)], {
      execArgv: ['--expose-gc', '--unhandled-rejections=strict'],
      env: { ...process.env, ...env }, stdio: ['ignore', 'inherit', 'inherit', 'ipc'],
    });
    let result;
    child.on('message', value => { result = value; });
    child.on('error', reject);
    const timeout = setTimeout(() => { child.kill(); reject(new Error(`Timed out: ${mode}`)); }, 600000);
    child.on('exit', (code, signal) => {
      clearTimeout(timeout);
      if (code || signal || !result) reject(new Error(`Failed: ${mode} ${code} ${signal}`));
      else resolve(result);
    });
  });
  const results = [];
  for (const fault of ['alloc', 'copy', 'copy-short', 'parse', 'span', 'install', 'capacity', 'original', 'borrowed', 'frozen', 'replaced', 'random']) {
    results.push(await run('callbacks', fault));
    console.log(`Callback child passed: ${fault}`);
  }
  const body = fixture();
  const gzip = gzipSync(body);
  let collectorBytes = 0, collectorRequests = 0, capturedBodies = 0;
  let collectorMode = 'healthy';
  let example;
  const server = createServer(async (req, res) => {
    if (req.url === '/v1/traces' || req.url === '/v1/metrics') {
      const chunks = []; for await (const part of req) chunks.push(part);
      const bytes = Buffer.concat(chunks);
      collectorBytes += bytes.length; collectorRequests++;
      // OTLP HTTP JSON is the distro's actual configured exporter format.
      assert.ok(!bytes.includes(Buffer.from(['http', 'canary'].join('-'))));
      const data = JSON.parse(bytes);
      for (const resource of data.resourceSpans || []) for (const scope of resource.scopeSpans || []) {
        for (const span of scope.spans || []) for (const attr of span.attributes || []) {
          if (attr.key === 'http.response.body') {
            capturedBodies++;
            example ??= attr.value.stringValue;
            assert.deepEqual(JSON.parse(attr.value.stringValue), {
              data: { product: { id: 42, name: 'Coffee', password: '****' } },
            });
          }
        }
      }
      if (collectorMode === 'slow') await new Promise(resolve => setTimeout(resolve, 250));
      if (collectorMode === 'unavailable') res.statusCode = 503;
      res.end('{}'); return;
    }
    const mode = Number(req.url.slice(1));
    const encoded = mode === 1 ? gzip : mode === 4 ? Buffer.from('bad gzip') : Buffer.from(body);
    res.writeHead(200, { 'content-type': 'application/graphql-response+json',
      ...(process.env.HTTP_CONTENT_LENGTH === '1' ? { 'content-length': encoded.length } : {}),
      ...(mode === 1 || mode === 4 ? { 'content-encoding': 'gzip' } : {}) });
    if (mode === 2 || mode === 3) {
      res.write(encoded.subarray(0, 5));
      setImmediate(() => res.end(encoded.subarray(5)));
    } else res.end(encoded);
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  try {
    const config = { port: server.address().port, count: Number(process.env.HTTP_BATCH || 1000) };
    const variants = process.env.HTTP_BASELINE
      ? [{ instrumented: true, module: process.env.HTTP_BASELINE }, { instrumented: true }]
      : [{ instrumented: false }, { instrumented: true }];
    for (const variant of variants) results.push(await run('http', { ...config, ...variant }));
    assert.equal(results.at(-1).digest, results.at(-2).digest);
    const bundles = (process.env.BUNDLES || path.join(root, 'opt/node/dist/init.mjs')).split(',');
    for (const bundle of bundles) for (const mode of ['healthy', 'slow', 'unavailable']) {
      collectorMode = mode;
      const before = capturedBodies;
      const beforeBytes = collectorBytes, beforeRequests = collectorRequests;
      results.push(await run('http', { ...config, count: 100, bundle }, {
        DASH0_EXTENSION_ENDPOINT: `http://127.0.0.1:${config.port}/v1/traces`,
        OTEL_EXPORTER_OTLP_TIMEOUT: '2000',
      }));
      Object.assign(results.at(-1), { collectorMode, capturedBodies: capturedBodies - before,
        collectorBytes: collectorBytes - beforeBytes, collectorRequests: collectorRequests - beforeRequests });
    }
    assert.ok(capturedBodies > 0);
    console.log(JSON.stringify({ results, collectorBytes, collectorRequests, capturedBodies, example }, null, 2));
  } finally {
    server.closeAllConnections();
    await new Promise(resolve => server.close(resolve));
  }
}
