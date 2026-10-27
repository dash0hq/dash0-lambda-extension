// Local, transport-level baseline/candidate benchmark. No AWS calls or deployments.
import assert from 'node:assert/strict';
import { fork, execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { existsSync, readFileSync, writeFileSync, mkdirSync } from 'node:fs';
import http from 'node:http';
import { Session } from 'node:inspector';
import { createRequire } from 'node:module';
import os from 'node:os';
import path from 'node:path';
import { monitorEventLoopDelay, performance } from 'node:perf_hooks';
import { fileURLToPath } from 'node:url';
import { gzipSync } from 'node:zlib';
import { writeHeapSnapshot } from 'node:v8';

const script = fileURLToPath(import.meta.url);
const root = path.resolve(path.dirname(script), '../..');
const require = createRequire(path.join(root, 'opt/node/package.json'));
const requestBody = JSON.stringify({ query: '{ product { id name password padding } }' });
const cases = [1024, 16384].flatMap(bytes => ['plain', 'gzip'].flatMap(encoding =>
  [1, 10].map(concurrency => ({ bytes, encoding, concurrency }))));
cases.push({ bytes: 65536, encoding: 'gzip', concurrency: 10 });
if (process.env.EXTENDED === '1') {
  cases.push(...[32767, 32768, 32769, 65535, 65537, 1048576].flatMap(bytes =>
    ['plain', 'gzip'].map(encoding => ({ bytes, encoding, concurrency: 50 }))));
}
const fixture = bytes => {
  const value = { data: { product: { id: 'p42', name: 'Coffee', password: 'benchmark-secret', padding: '' } } };
  value.data.product.padding = 'x'.repeat(bytes - Buffer.byteLength(JSON.stringify(value)));
  return value;
};
const percentile = (sorted, p) => sorted[Math.max(0, Math.ceil(sorted.length * p) - 1)];
const sha = file => createHash('sha256').update(readFileSync(file)).digest('hex');

if (process.argv[2] === 'server') {
  const responses = new Map(cases.map(c => {
    const body = Buffer.from(JSON.stringify(fixture(c.bytes)));
    return [`/${c.bytes}/${c.encoding}`, {
      body: c.encoding === 'gzip' ? gzipSync(body) : body,
      encoding: c.encoding,
    }];
  }));
  const server = http.createServer(async (req, res) => {
    const parts = [];
    for await (const part of req) parts.push(part);
    assert.equal(Buffer.concat(parts).toString(), requestBody);
    const response = responses.get(req.url);
    assert.ok(response);
    res.writeHead(200, {
      'content-type': 'application/json',
      'content-length': response.body.length,
      ...(response.encoding === 'gzip' ? { 'content-encoding': 'gzip' } : {}),
    });
    res.end(response.body);
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  process.send({ port: server.address().port });
  process.on('message', message => {
    if (message === 'stop') {
      server.closeAllConnections();
      server.close(() => process.disconnect());
    }
  });
} else if (process.argv[2] === 'worker') {
  const config = JSON.parse(process.argv[3]);
  const expectedProducts = new Map(cases.map(c => [c.bytes, fixture(c.bytes).data.product]));
  let provider;
  let instrumentation;
  let bodyForCase;
  const exported = { spans: 0, requestBodies: 0, responseBodies: 0, attributeBytes: 0 };
  if (config.variant.startsWith('instrumented')) {
    const { NodeTracerProvider } = require('@opentelemetry/sdk-trace-node');
    // A bounded attribute sink: measures span production/serialization, not OTLP or extension cost.
    provider = new NodeTracerProvider({ spanProcessors: [{
      onStart() {},
      onEnd(span) {
        // Diagnostic lower bound for exporting the extra payload: append a known
        // fixture in the sink, without capture/decode/masking or SDK setAttribute.
        // Never enable this mode for the original end-to-end gate.
        if (config.variant === 'instrumented-export-only' && bodyForCase !== undefined) {
          span.attributes['http.response.body'] = bodyForCase;
        }
        exported.spans++;
        if (typeof span.attributes['http.request.body'] === 'string') exported.requestBodies++;
        if (typeof span.attributes['http.response.body'] === 'string') {
          exported.responseBodies++;
          assert.ok(!span.attributes['http.response.body'].includes('benchmark-secret'));
        }
        exported.attributeBytes += Buffer.byteLength(JSON.stringify(span.attributes));
      },
      async forceFlush() {},
      async shutdown() {},
    }] });
    provider.register();
    const Instrumentation = require(['instrumented-before', 'instrumented-export-only'].includes(config.variant)
      ? process.env.BASELINE_MODULE : config.variant === 'instrumented-first' ? process.env.FIRST_CANDIDATE_MODULE : path.join(root,
        'opt/node/distro/dist/src/instrumentations/undici/UndiciInstrumentation.js')).default;
    instrumentation = new Instrumentation().getInstrumentation();
    instrumentation.setTracerProvider(provider);
    instrumentation.enable();
  }
  const post = async c => {
    const response = await fetch(`http://127.0.0.1:${config.port}/${c.bytes}/${c.encoding}`, {
      method: 'POST', headers: { 'content-type': 'application/json' }, body: requestBody,
    });
    assert.equal(response.status, 200);
    const product = (await response.json()).data.product;
    assert.equal(product.id, 'p42');
    assert.equal(product.name, 'Coffee');
    assert.equal(product.password, 'benchmark-secret');
    assert.equal(product.padding, expectedProducts.get(c.bytes).padding);
  };
  const run = async (c, count, samples) => {
    let next = 0;
    await Promise.all(Array.from({ length: c.concurrency }, async () => {
      while (next++ < count) {
        const start = performance.now();
        await post(c);
        samples?.push(performance.now() - start);
      }
    }));
  };
  const results = [];
  const selectedCases = process.env.CASE_INDEXES
    ? process.env.CASE_INDEXES.split(',').map(index => cases[Number(index)]) : cases;
  assert.ok(selectedCases.every(Boolean), 'Invalid CASE_INDEXES');
  const selected = config.profile ? [cases[Number(process.env.PROFILE_CASE || 7)]] :
    (config.reverse ? [...selectedCases].reverse() : selectedCases);
  let profiler;
  const command = (method, args = {}) => new Promise((resolve, reject) =>
    profiler.post(method, args, (error, result) => error ? reject(error) : resolve(result)));
  try {
    if (config.profile) writeHeapSnapshot(`${config.profile}.before.heapsnapshot`);
    for (const c of selected) {
      const expected = fixture(c.bytes);
      expected.data.product.password = '****';
      const limit = Math.min(Number(process.env.OTEL_SPAN_ATTRIBUTE_VALUE_LENGTH_LIMIT || 4096), 65536);
      bodyForCase = c.bytes <= limit && (c.encoding === 'gzip' || c.bytes <= 32768)
        ? JSON.stringify(expected) : undefined;
      await run(c, config.warmup);
      global.gc();
      if (config.profile) {
        profiler = new Session();
        profiler.connect();
        await command('Profiler.enable');
        await command('Profiler.setSamplingInterval', { interval: 1000 });
        await command('Profiler.start');
        await command('HeapProfiler.startSampling', { samplingInterval: 32768,
          includeObjectsCollectedByMajorGC: true, includeObjectsCollectedByMinorGC: true });
      }
      const before = process.memoryUsage();
      let peakRss = before.rss;
      let peakExternal = before.external;
      const memorySampler = setInterval(() => {
        const m = process.memoryUsage();
        peakRss = Math.max(peakRss, m.rss);
        peakExternal = Math.max(peakExternal, m.external);
      }, 10);
      const delay = monitorEventLoopDelay({ resolution: 10 });
      delay.enable();
      const utilization = performance.eventLoopUtilization();
      const beforeExport = { ...exported };
      const cpuStart = process.cpuUsage();
      const start = performance.now();
      const samples = [];
      await run(c, config.requests, samples);
      const elapsedMs = performance.now() - start;
      const cpu = process.cpuUsage(cpuStart);
      const elu = performance.eventLoopUtilization(utilization);
      clearInterval(memorySampler);
      delay.disable();
      if (profiler) {
        const allocation = await command('HeapProfiler.stopSampling');
        writeFileSync(`${config.profile}.allocations.json`, JSON.stringify(allocation));
        const { profile } = await command('Profiler.stop');
        writeFileSync(config.profile, JSON.stringify(profile));
        profiler.disconnect();
        profiler = undefined;
      }
      const after = process.memoryUsage();
      peakRss = Math.max(peakRss, after.rss);
      peakExternal = Math.max(peakExternal, after.external);
      await provider?.forceFlush();
      const counts = Object.fromEntries(Object.keys(exported).map(key => [key, exported[key] - beforeExport[key]]));
      if (config.variant.startsWith('instrumented')) {
        assert.equal(counts.spans, config.requests);
        assert.equal(counts.requestBodies, config.requests);
        if (config.variant === 'instrumented-before') assert.equal(counts.responseBodies, 0);
        if (['instrumented-after', 'instrumented-first', 'instrumented-export-only'].includes(config.variant)) assert.equal(counts.responseBodies,
          bodyForCase === undefined ? 0 : config.requests);
      }
      global.gc();
      samples.sort((a, b) => a - b);
      results.push({ ...c, requests: samples.length, elapsedMs,
        requestsPerSecond: config.requests / elapsedMs * 1000,
        latencyMs: { p50: percentile(samples, .5), p95: percentile(samples, .95), p99: percentile(samples, .99) },
        latencySamplesMs: samples,
        cpuMicrosecondsPerRequest: (cpu.user + cpu.system) / config.requests,
        eventLoopUtilization: elu.utilization,
        eventLoopDelayP99Ms: delay.count ? delay.percentile(99) / 1e6 : null,
        memoryBytes: { before, after, afterGc: process.memoryUsage(), sampledPeakRss: peakRss, sampledPeakExternal: peakExternal },
        exported: counts,
      });
    }
    if (config.profile) {
      global.gc();
      writeHeapSnapshot(`${config.profile}.after.heapsnapshot`);
    }
    process.send({ variant: config.variant, profile: config.profile, cases: results });
  } finally {
    instrumentation?.disable();
    await provider?.shutdown();
  }
} else {
  const output = path.resolve(process.argv[2] ?? path.join(root, '.amp/in/artifacts/undici-before'));
  const rounds = Number(process.env.ROUNDS ?? 10);
  const requests = Number(process.env.REQUESTS ?? 300);
  const warmup = Number(process.env.WARMUP ?? 100);
  assert.ok(Number.isInteger(rounds) && rounds > 0);
  assert.ok(Number.isInteger(requests) && requests > 0);
  assert.ok(Number.isInteger(warmup) && warmup > 0);
  mkdirSync(path.dirname(output), { recursive: true });
  const server = fork(script, ['server'], { stdio: ['ignore', 'inherit', 'inherit', 'ipc'] });
  const port = await new Promise((resolve, reject) => {
    server.once('message', value => resolve(value.port));
    server.once('error', reject);
    server.once('exit', code => reject(new Error(`Server exited before ready: ${code}`)));
  });
  const worker = config => new Promise((resolve, reject) => {
    let result;
    const child = fork(script, ['worker', JSON.stringify({ port, requests, warmup, ...config })], {
      execArgv: ['--expose-gc'], stdio: ['ignore', 'inherit', 'inherit', 'ipc'],
    });
    child.on('message', value => { result = value; });
    child.on('error', reject);
    child.on('exit', (code, signal) => {
      if (code !== 0 || signal || !result) reject(new Error(`Worker failed: code=${code}, signal=${signal}`));
      else resolve(result);
    });
  });
  const results = [];
  const variants = process.env.BASELINE_MODULE
    ? ['instrumented-before', 'instrumented-after'] : ['uninstrumented', 'instrumented'];
  if (process.env.FIRST_CANDIDATE_MODULE) variants.splice(1, 0, 'instrumented-first');
  if (process.env.EXPORT_BODY_ONLY === '1') {
    assert.ok(process.env.BASELINE_MODULE, 'EXPORT_BODY_ONLY requires BASELINE_MODULE');
    variants.splice(1, 0, 'instrumented-export-only');
  }
  try {
    for (let round = 0; round < rounds; round++) {
      let order = round % 2 ? [...variants].reverse() : [...variants];
      if (variants.length > 2) {
        const shift = Math.floor(round / 2) % variants.length;
        order = [...order.slice(shift), ...order.slice(0, shift)];
      }
      for (const variant of order) {
        const result = await worker({ variant, reverse: !!(round % 2) });
        results.push({ round, ...result });
        console.log(`Round ${round + 1}/${rounds}: ${variant}, ${result.cases.length} cases verified`);
      }
    }
    const profiles = [];
    for (const variant of variants) {
      profiles.push(await worker({ variant, requests: 10000,
        profile: `${output}-${variant}.cpuprofile` }));
      console.log(`CPU profile captured: ${variant}`);
    }
    const manifest = {
      measuredAt: new Date().toISOString(), gitHead: execFileSync('git', ['rev-parse', 'HEAD'], { cwd: root, encoding: 'utf8' }).trim(),
      runtime: { node: process.version, undici: process.versions.undici, platform: process.platform, arch: process.arch,
        cpu: os.cpus()[0].model, logicalCpus: os.cpus().length },
      hashes: { harness: sha(script), lockfile: sha(path.join(root, 'opt/node/package-lock.json')),
        instrumentation: sha(path.join(root, 'opt/node/distro/dist/src/instrumentations/undici/UndiciInstrumentation.js')),
        responseCapture: existsSync(path.join(root, 'opt/node/distro/dist/src/instrumentations/undici/responseBody.js'))
          ? sha(path.join(root, 'opt/node/distro/dist/src/instrumentations/undici/responseBody.js')) : undefined,
        firstCandidate: process.env.FIRST_CANDIDATE_MODULE ? sha(process.env.FIRST_CANDIDATE_MODULE) : undefined,
        firstResponseCapture: process.env.FIRST_CANDIDATE_MODULE
          ? sha(path.join(path.dirname(process.env.FIRST_CANDIDATE_MODULE), 'responseBody.js')) : undefined,
        baseline: process.env.BASELINE_MODULE ? sha(process.env.BASELINE_MODULE) : undefined },
      scope: 'Local real fetch and Undici instrumentation with a synchronous, non-retaining attribute serialization sink. Not the full distro, OTLP exporter, Rust extension, or AWS Lambda. Timing excludes startup/warmup; profiling runs are separate. Memory peaks are sampled every 10ms; cases share a worker process.',
      rounds, requests, warmup, caseIndexes: process.env.CASE_INDEXES, exportOnlyDiagnostic: process.env.EXPORT_BODY_ONLY === '1', results, profiles,
    };
    writeFileSync(`${output}.json`, JSON.stringify(manifest, null, 2) + '\n');
    console.log(`Raw results: ${output}.json`);
  } finally {
    server.send('stop');
    await new Promise(resolve => server.once('exit', resolve));
  }
}
