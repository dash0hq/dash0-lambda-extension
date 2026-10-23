// Ad-hoc: where does json_4k time go? Run with --cpu-prof.
const fx = require('../../../benches/fixtures/masking-payloads.json');
const { ScrubContext } = require('@lumigo/node-core');
const { scrubHttpPayload } = require('./dist/src/tools/payloads');
const b = fx.payloads.json_4k;
let t = process.hrtime.bigint();
for (let i = 0; i < 20000; i++) JSON.stringify(JSON.parse(b));
console.log('parse+stringify ns', Number(process.hrtime.bigint() - t) / 20000);
t = process.hrtime.bigint();
for (let i = 0; i < 20000; i++) scrubHttpPayload(b, 'application/json', ScrubContext.HTTP_REQUEST_BODY);
console.log('scrubHttpPayload ns', Number(process.hrtime.bigint() - t) / 20000);
