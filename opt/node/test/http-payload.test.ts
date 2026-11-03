/*
 * Reproduction and regression test for a customer report: a GraphQL client on the
 * http/https modules (axios, node-fetch v2, got) received a gzip response, and the
 * `http_response_body` payload log contained the gzip bytes decoded as UTF-8:
 *
 *   {"message":"\u001f�\b\u0000\u0000\u0000\u0000\u0000\u0004�lO�n�0\f~...","type":"http_response_body"}
 *
 * The response carried `content-type: application/json` and `content-encoding: gzip`.
 * Node's http module never decompresses; clients pipe the response through zlib
 * themselves. The http hook turned each raw chunk into a string, so the bytes were lost
 * before JSON parsing and masking, and the garbage was exported unmasked.
 *
 * Each run is a separate process (see `http-payload/runner.mjs`): require-in-the-middle
 * cannot patch `http` inside jest's module loader.
 */

import { execFileSync } from 'child_process';
import * as path from 'path';
import { randomBytes } from 'crypto';
import { brotliCompressSync, deflateSync, gzipSync } from 'zlib';

const ROOT = path.join(__dirname, '..');
const RUNNER = path.join(__dirname, 'http-payload', 'runner.mjs');

interface Scenario {
  encoding?: string;
  wire: Buffer;
  chunked?: boolean;
  contentType?: string;
  setEncoding?: BufferEncoding;
}
interface Result {
  appBodyLength: number;
  responseBody?: string;
  responseHeaders: string;
}

function run(scenarios: Scenario[]): Result[] {
  const stdout = execFileSync(
    process.execPath,
    [RUNNER],
    {
      // Scenarios go over stdin: one argv string is capped at 128 KiB on Linux.
      input: JSON.stringify(scenarios.map(s => ({ ...s, wire: s.wire.toString('base64') }))),
      encoding: 'utf8', stdio: ['pipe', 'pipe', 'inherit'], cwd: ROOT, timeout: 60_000,
    }
  );
  const line = stdout.split('\n').find(l => l.startsWith('RESULT '));
  if (!line) throw new Error(`runner printed no result:\n${stdout}`);
  return JSON.parse(line.slice('RESULT '.length));
}

const graphqlResponse = {
  data: { order: { id: 'order-42', status: 'DELIVERED', items: [{ name: 'Chicken Bucket', qty: 1 }] } },
  extensions: { accessToken: 'response-canary' },
};
const masked = JSON.stringify({ ...graphqlResponse, extensions: { accessToken: '****' } });
const json = (value: unknown) => Buffer.from(JSON.stringify(value));
// The secret comes first, so the truncated capture proves it was masked rather than cut away.
const large = { password: 'hidden', items: Array.from({ length: 400 }, (_, i) => ({ id: i, name: 'Chicken Bucket' })) };
// Random base64 barely compresses: about 36 KiB on the wire, 48 KiB decoded.
const incompressible = { blob: randomBytes(36_000).toString('base64') };

const scenarios: [string, Scenario, (result: Result) => void][] = [
  ['gzip GraphQL response (customer reproduction)', { encoding: 'gzip', wire: gzipSync(json(graphqlResponse)) },
    result => {
      expect(JSON.parse(result.responseHeaders)).toMatchObject({ 'content-encoding': 'gzip' });
      expect(result.responseBody).toBe(masked);
    }],
  ...['gzip', 'x-gzip', 'deflate', 'br'].map((encoding): [string, Scenario, (result: Result) => void] => [
    `${encoding} across several network chunks`,
    {
      encoding, chunked: true,
      wire: (encoding === 'br' ? brotliCompressSync : encoding === 'deflate' ? deflateSync : gzipSync)(json(graphqlResponse)),
    },
    result => expect(result.responseBody).toBe(masked),
  ]),
  ['identity response, unchanged', { wire: json(graphqlResponse), chunked: true },
    result => expect(result.responseBody).toBe(masked)],
  // Same structural truncation as any oversized parsed http payload (see masking-env.test.ts).
  ['large gzip response, decoded, masked, then truncated', { encoding: 'gzip', wire: gzipSync(json(large)) },
    result => expect(result.responseBody).toBe('{"password":"****","items":"✂"}')],
  ['gzip body past the 32 KiB wire limit', { encoding: 'gzip', wire: gzipSync(json(incompressible)) },
    result => expect(result.responseBody).toBeUndefined()],
  ['gzip text/html is decoded and captured as text',
    { encoding: 'gzip', contentType: 'text/html', wire: gzipSync(Buffer.from('<p>Order 42 delivered</p>')) },
    result => expect(result.responseBody).toBe(JSON.stringify('<p>Order 42 delivered</p>'))],
  ['client calls setEncoding() on a gzip response',
    { encoding: 'gzip', setEncoding: 'latin1', wire: gzipSync(json(graphqlResponse)) },
    result => expect(result.responseBody).toBeUndefined()],
  ['header value naming an Object.prototype member',
    { encoding: 'constructor', wire: json(graphqlResponse) },
    result => expect(result.responseBody).toBeUndefined()],
  ['corrupt gzip', { encoding: 'gzip', wire: Buffer.from([0x1f, 0x8b, 8, 0, 1, 2, 3, 4, 5, 6]) },
    result => expect(result.responseBody).toBeUndefined()],
  ['unsupported encoding', { encoding: 'zstd', wire: Buffer.from([0x28, 0xb5, 0x2f, 0xfd, 0, 1, 2, 3]) },
    result => expect(result.responseBody).toBeUndefined()],
  ['gzip bomb', { encoding: 'gzip', wire: gzipSync(json({ padding: 'x'.repeat(1024 * 1024) })) },
    result => expect(result.responseBody).toBeUndefined()],
];

let results: Result[];

beforeAll(() => {
  // The runner loads the compiled distro; CI runs `npm test` without `npm run build`.
  execFileSync(
    process.execPath,
    [require.resolve('typescript/bin/tsc'), '-p', 'distro/tsconfig.json', '--skipLibCheck'],
    { cwd: ROOT, stdio: 'inherit', timeout: 90_000 }
  );
  results = run(scenarios.map(([, scenario]) => scenario));
}, 120_000);

describe('http/https response payload capture', () => {
  test.each(scenarios.map(([name], i) => [name, i] as const))('%s', (_name, i) => {
    const [, scenario, check] = scenarios[i];
    // The application still receives its response; capture never consumes it.
    if (!['zstd', 'constructor'].includes(scenario.encoding!) && !scenario.wire.equals(Buffer.from([0x1f, 0x8b, 8, 0, 1, 2, 3, 4, 5, 6]))) {
      expect(results[i].appBodyLength).toBeGreaterThan(0);
    }
    check(results[i]);
    // Compressed bytes are never exported as text.
    expect(results[i].responseBody ?? '').not.toMatch(/\u001f|�/);
  });
});
