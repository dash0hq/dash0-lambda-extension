// Prints what the distro emits for each fixture, plus which secrets survive.
const fs = require('fs');
const path = require('path');
const ROOT = path.resolve(__dirname, '..', '..', '..');
const fx = JSON.parse(fs.readFileSync(path.join(ROOT, 'benches/fixtures/masking-payloads.json'), 'utf8'));

const { CommonUtils, ScrubContext } = require('@lumigo/node-core');
const { scrubHttpPayload } = require('./dist/src/tools/payloads');
const { Http } = require('./dist/src/instrumentations/https/http');
const { getAbsoluteUrl } = require('@opentelemetry/instrumentation-http/build/src/utils');

const CT = {
  small_json: 'application/json', json_4k: 'application/json', truncated_json_4k: 'application/json',
  non_matching_json_4k: 'application/json', form_encoded: 'application/x-www-form-urlencoded', plain_text: 'text/plain',
};
const SECRETS = ['hunter2', 't-123', 'jane@example.com', 'Bearer abc', 'k-1', 's-1', 'abc'];
const leaks = (s) => SECRETS.filter((x) => s && s.includes(x));
const show = (label, input, s) =>
  console.log(`${label}\n  in  ${input.length} chars, out ${s ? s.length : s} chars, '****' x${(s || '').split('****').length - 1}, still visible: ${JSON.stringify(leaks(s))}\n  out[0..160]: ${JSON.stringify((s || '').slice(0, 160))}\n  out[-60..]: ${JSON.stringify((s || '').slice(-60))}`);

console.log(`LUMIGO_SECRET_MASKING_REGEX=${process.env.LUMIGO_SECRET_MASKING_REGEX}\n`);
for (const [name, body] of Object.entries(fx.payloads)) {
  show(`request body ${name} (${CT[name]})`, body, scrubHttpPayload(body, CT[name], ScrubContext.HTTP_REQUEST_BODY));
}

// Response bodies go through the capture path first: http.ts stops buffering at the attribute limit.
for (const name of ['json_4k', 'small_json']) {
  const body = fx.payloads[name];
  const span = { attributes: {}, setAttribute(k, v) { this.attributes[k] = v; } };
  const response = { headers: { 'content-type': 'application/json' }, statusCode: 200 };
  const onEmit = Http.createEmitResponseOnEmitBeforeHookHandler({ request: {}, response: {} }, response, Http.onRequestEnd(span));
  onEmit(['data', Buffer.from(body)]);
  onEmit(['end']);
  show(`response body ${name} via capture path`, body, span.attributes['http.response.body']);
}

const a = fx.span_attributes.json_4k;
show('http.request.headers', a['http.request.headers'], CommonUtils.payloadStringify(JSON.parse(a['http.request.headers']), ScrubContext.HTTP_REQUEST_HEADERS, 4096));
show('http.response.headers', a['http.response.headers'], CommonUtils.payloadStringify(JSON.parse(a['http.response.headers']), ScrubContext.HTTP_RESPONSE_HEADERS, 4096));
const u = new URL(a['url.full']);
show('url.full (upstream getAbsoluteUrl)', a['url.full'], getAbsoluteUrl({ protocol: u.protocol, host: u.host, path: u.pathname + u.search }, {}, 'https:'));
