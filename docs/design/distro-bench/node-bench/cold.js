// Cold cost in a fresh process: module load (Lumigo builds and compiles its 7 scrubbers at
// require time) and the first masking calls, before the JIT has warmed up.
const fs = require('fs');
const path = require('path');
const ROOT = path.resolve(__dirname, '..', '..', '..');
const fx = JSON.parse(fs.readFileSync(path.join(ROOT, 'benches/fixtures/masking-payloads.json'), 'utf8'));
const ns = (t0) => Number(process.hrtime.bigint() - t0);

let t = process.hrtime.bigint();
const { ScrubContext } = require('@lumigo/node-core');
const requireLumigo = ns(t);

t = process.hrtime.bigint();
const { scrubHttpPayload } = require('./dist/src/tools/payloads');
const requirePayloads = ns(t);

t = process.hrtime.bigint();
scrubHttpPayload(fx.payloads.small_json, 'application/json', ScrubContext.HTTP_REQUEST_BODY);
const firstSmall = ns(t);

t = process.hrtime.bigint();
scrubHttpPayload(fx.payloads.json_4k, 'application/json', ScrubContext.HTTP_REQUEST_BODY);
const firstJson4k = ns(t);

t = process.hrtime.bigint();
scrubHttpPayload(fx.payloads.json_4k, 'application/json', ScrubContext.HTTP_REQUEST_BODY);
const secondJson4k = ns(t);

console.log(JSON.stringify({ mode: process.env.BENCH_MODE, requireLumigo, requirePayloads, firstSmall, firstJson4k, secondJson4k }));
