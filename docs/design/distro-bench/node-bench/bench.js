// Micro-benchmark of the Node distro's payload masking, calling the real compiled distro code.
// Run through run.sh, which maps DASH0_MASK_* with the real opt/shared.sh before node starts.
const fs = require('fs');
const path = require('path');
const { PerformanceObserver } = require('perf_hooks');

const ROOT = path.resolve(__dirname, '..', '..', '..');
const fx = JSON.parse(fs.readFileSync(path.join(ROOT, 'benches/fixtures/masking-payloads.json'), 'utf8'));

const { CommonUtils, ScrubContext } = require('@lumigo/node-core');
const { scrubHttpPayload, contentType } = require('./dist/src/tools/payloads');
const { HttpHooks, Http } = require('./dist/src/instrumentations/https/http');
const { getSpanAttributeMaxLength } = require('./dist/src/utils');
// url.full of an HTTP client span is built (and query-redacted) by upstream instrumentation-http.
const { getAbsoluteUrl } = require('@opentelemetry/instrumentation-http/build/src/utils');

const MODE = process.env.BENCH_MODE || 'default';
const MAX = getSpanAttributeMaxLength();

const CT = {
  small_json: 'application/json',
  json_4k: 'application/json',
  truncated_json_4k: 'application/json',
  non_matching_json_4k: 'application/json',
  form_encoded: 'application/x-www-form-urlencoded',
  plain_text: 'text/plain',
};

const fakeSpan = () => ({ attributes: {}, setAttribute(k, v) { this.attributes[k] = v; } });
const urlParts = (u) => {
  const p = new URL(u);
  return { protocol: p.protocol, host: p.host, path: p.pathname + p.search };
};

// Per payload: the call the HTTP hooks make for a request body.
const payloadCases = Object.keys(fx.payloads).map((name) => {
  const body = fx.payloads[name];
  const ct = CT[name];
  return { name, fn: () => scrubHttpPayload(body, ct, ScrubContext.HTTP_REQUEST_BODY) };
});
{
  const parts = urlParts(fx.span_attributes.small_json['url.full']);
  payloadCases.push({ name: 'url_query (upstream getAbsoluteUrl)', fn: () => getAbsoluteUrl(parts, {}, 'https:') });
}

// Per span: every payload attribute of one HTTP client span, masked the way http.ts does it.
const spanCase = (name) => {
  const a = fx.span_attributes[name];
  // At capture time the hooks see header objects, not the JSON strings stored on the span.
  const reqHeaders = JSON.parse(a['http.request.headers']);
  const resHeaders = JSON.parse(a['http.response.headers']);
  const parts = urlParts(a['url.full']);
  return () => {
    const span = fakeSpan();
    span.setAttribute('url.full', getAbsoluteUrl(parts, {}, 'https:'));
    span.setAttribute(
      'http.request.headers',
      CommonUtils.payloadStringify(reqHeaders, ScrubContext.HTTP_REQUEST_HEADERS, MAX)
    );
    span.setAttribute(
      'http.request.body',
      scrubHttpPayload(a['http.request.body'], contentType(reqHeaders), ScrubContext.HTTP_REQUEST_BODY)
    );
    HttpHooks.responseHook(span, { headers: resHeaders });
    Http.onRequestEnd(span)(
      { request: {}, response: {} },
      { body: a['http.response.body'], headers: resHeaders, statusCode: 200, truncated: false }
    );
    return span;
  };
};
const spanCases = Object.keys(fx.span_attributes).map((name) => ({ name, fn: spanCase(name) }));
const invocationCases = spanCases.map(({ name, fn }) => ({
  name,
  fn: () => {
    for (let i = 0; i < fx.spans_per_invocation; i++) fn();
  },
}));

// Timing: samples of `batch` calls each, so the ~30ns hrtime overhead stays negligible.
const pct = (sorted, p) => sorted[Math.min(sorted.length - 1, Math.floor((p / 100) * sorted.length))];
function time(fn) {
  const t0 = process.hrtime.bigint();
  let n = 0;
  while (process.hrtime.bigint() - t0 < 300_000_000n) { fn(); n++; } // warm-up 300ms
  const perCall = 300_000_000 / n;
  const batch = Math.max(1, Math.round(20_000 / perCall)); // ~20us per sample
  const samples = [];
  const end = process.hrtime.bigint() + 1_500_000_000n;
  while (process.hrtime.bigint() < end) {
    const s = process.hrtime.bigint();
    for (let i = 0; i < batch; i++) fn();
    samples.push(Number(process.hrtime.bigint() - s) / batch);
  }
  samples.sort((x, y) => x - y);
  return { p50: pct(samples, 50), p99: pct(samples, 99), samples: samples.length, batch };
}

// Allocation: heapUsed delta over K calls with a large young generation and no GC in between.
let gcs = 0;
new PerformanceObserver((l) => (gcs += l.getEntries().length)).observe({ entryTypes: ['gc'] });
const tick = () => new Promise((r) => setTimeout(r, 20)); // let the gc observer report first
async function alloc(fn, k) {
  global.gc();
  await tick();
  const g0 = gcs;
  const before = process.memoryUsage().heapUsed;
  for (let i = 0; i < k; i++) fn();
  const after = process.memoryUsage().heapUsed;
  await tick();
  return gcs === g0 ? (after - before) / k : NaN; // NaN: a GC ran mid-loop, the delta is meaningless
}

const fmt = (ns) => (ns >= 1000 ? `${(ns / 1000).toFixed(2)} µs` : `${ns.toFixed(0)} ns`);

(async () => {
  const out = { mode: MODE, env: process.env.LUMIGO_SECRET_MASKING_REGEX, node: process.version, max: MAX, payload: [], span: [], invocation: [] };
  for (const [key, cases, k] of [['payload', payloadCases, 2000], ['span', spanCases, 1000], ['invocation', invocationCases, 50]]) {
    for (const { name, fn } of cases) {
      const t = time(fn);
      let bytes = NaN;
      for (let kk = k; Number.isNaN(bytes) && kk >= 1; kk = Math.floor(kk / 4)) bytes = await alloc(fn, kk);
      out[key].push({ name, ...t, bytesPerCall: bytes });
      console.error(`${MODE} ${key} ${name}: p50 ${fmt(t.p50)} p99 ${fmt(t.p99)} alloc ${Math.round(bytes)} B`);
    }
  }
  console.log(JSON.stringify(out));
})();
