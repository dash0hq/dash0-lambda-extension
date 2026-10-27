// Diagnostic phase costs, not a replacement for the real-fetch end-to-end gate.
import assert from 'node:assert/strict';
import { fork } from 'node:child_process';
import { createHash } from 'node:crypto';
import { readFileSync, writeFileSync, mkdirSync } from 'node:fs';
import { Session } from 'node:inspector';
import { createRequire } from 'node:module';
import path from 'node:path';
import { performance } from 'node:perf_hooks';
import { fileURLToPath } from 'node:url';
import { gzipSync } from 'node:zlib';

const script = fileURLToPath(import.meta.url);
const root = path.resolve(path.dirname(script), '../..');
const require = createRequire(path.join(root, 'opt/node/package.json'));
const phases = ['control', 'setup-cleanup', 'copy-cleanup', 'capture', 'capture-serialize'];
if (process.argv[2] === 'worker') {
  const config = JSON.parse(process.argv[3]);
  const { captureResponseBody } = require(config.module);
  class Request {
    onData() { return false; }
    onComplete() { return 823; }
    onError() { return 823; }
  }
  const text = JSON.stringify({ data: { password: 'cost-canary', padding: 'x'.repeat(960) } });
  const expected = JSON.stringify({ data: { password: '****', padding: 'x'.repeat(960) } });
  const attributes = { 'http.request.body': '{"query":"{ product { id password } }"}' };
  let body, serialized;
  const span = { isRecording: () => true, setAttribute: (_key, value) => { body = value; } };
  const results = [];
  for (const encoding of ['identity', 'gzip']) for (const phase of config.reverse ? [...phases].reverse() : phases) {
    const bytes = encoding === 'gzip' ? gzipSync(text) : Buffer.from(text);
    const headers = { 'content-type': 'application/json', 'content-encoding': encoding,
      'content-length': String(bytes.length) };
    const run = count => {
      for (let i = 0; i < count; i++) {
        const request = new Request();
        if (phase !== 'control') captureResponseBody(request, span, headers);
        if (phase === 'control' || phase === 'setup-cleanup') request.onError();
        else {
          request.onData(bytes);
          if (phase === 'copy-cleanup') request.onError();
          else {
            request.onComplete();
            if (phase === 'capture-serialize') serialized = JSON.stringify({ ...attributes, 'http.response.body': body });
          }
        }
      }
    };
    run(1000); global.gc();
    let profiler;
    const command = (name, args = {}) => new Promise((resolve, reject) =>
      profiler.post(name, args, (error, result) => error ? reject(error) : resolve(result)));
    if (config.profile) {
      profiler = new Session(); profiler.connect();
      await command('Profiler.enable'); await command('Profiler.start');
      await command('HeapProfiler.startSampling', { samplingInterval: 32768,
        includeObjectsCollectedByMajorGC: true, includeObjectsCollectedByMinorGC: true });
    }
    body = undefined;
    const cpu = process.cpuUsage(), start = performance.now();
    run(config.count);
    const elapsedMs = performance.now() - start, used = process.cpuUsage(cpu);
    if (phase === 'capture' || phase === 'capture-serialize') assert.equal(body, expected);
    if (phase === 'capture-serialize') assert.equal(JSON.parse(serialized)['http.response.body'], expected);
    if (profiler) {
      writeFileSync(`${config.profile}-${encoding}-${phase}.allocations.json`, JSON.stringify(await command('HeapProfiler.stopSampling')));
      writeFileSync(`${config.profile}-${encoding}-${phase}.cpuprofile`, JSON.stringify((await command('Profiler.stop')).profile));
      profiler.disconnect();
    }
    results.push({ phase, encoding, count: config.count, elapsedMs,
      cpuMicrosecondsPerCycle: (used.user + used.system) / config.count });
  }
  process.send({ ...config, results });
} else {
  const prefix = path.resolve(process.argv[2] || '/tmp/undici-costs');
  mkdirSync(path.dirname(prefix), { recursive: true });
  const modules = { before: process.env.CAPTURE_BEFORE,
    after: path.join(root, 'opt/node/distro/dist/src/instrumentations/undici/responseBody.js') };
  assert.ok(modules.before, 'Set CAPTURE_BEFORE to the frozen first candidate responseBody.js');
  const run = config => new Promise((resolve, reject) => {
    let result;
    const child = fork(script, ['worker', JSON.stringify(config)], {
      execArgv: ['--expose-gc'], stdio: ['ignore', 'inherit', 'inherit', 'ipc'],
    });
    child.on('message', value => { result = value; }); child.on('error', reject);
    child.on('exit', (code, signal) => code || signal || !result
      ? reject(new Error(`Diagnostic child failed: ${code} ${signal}`)) : resolve(result));
  });
  const results = [];
  for (let round = 0; round < Number(process.env.ROUNDS || 5); round++) {
    for (const variant of round % 2 ? ['after', 'before'] : ['before', 'after']) {
      results.push(await run({ round, variant, module: modules[variant], reverse: !!(round % 2), count: 20000 }));
    }
  }
  const profiles = [];
  for (const variant of ['before', 'after']) profiles.push(await run({ variant, module: modules[variant], count: 10000,
    profile: `${prefix}-${variant}` }));
  writeFileSync(`${prefix}.json`, JSON.stringify({
    scope: 'Synthetic inherited Request callbacks and minimal span/sink. Setup+error cleanup; add copy; add decode/parse/mask/setAttribute; add minimal attribute serialization. Phase differences are diagnostic, not additive exact causal attribution or the real-fetch export gate. Native buffer allocations are not fully represented by heap sampling.',
    node: process.version,
    hashes: Object.fromEntries(Object.entries(modules).map(([variant, file]) =>
      [variant, createHash('sha256').update(readFileSync(file)).digest('hex')])),
    results, profiles,
  }, null, 2) + '\n');
}
