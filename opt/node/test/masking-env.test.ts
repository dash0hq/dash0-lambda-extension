/*
 * SIG-695 / SIG-699: the documented DASH0_MASK_* env vars must control masking of the
 * payload attributes the Node distro captures.
 *
 * The distro masks through @lumigo/node-core, which only reads LUMIGO_SECRET_MASKING_REGEX*
 * and only once, when it is first required. `opt/shared.sh` therefore maps DASH0_MASK_*
 * onto those variables in the exec wrapper, before the runtime starts.
 *
 * The mapping itself is tested, language-neutrally, in opt/test/shared.test.sh. This suite
 * checks the Node side: each scenario runs the real `map_masking_env` in bash, loads the
 * resulting environment into a fresh module registry, and drives the real express and
 * aws-sdk hooks. The express hook covers every HTTP context (query, request/response
 * headers, request/response body); the aws-sdk hooks cover the SQS publish and consume
 * bodies.
 */

import { execFileSync } from 'child_process';
import * as path from 'path';
import { BasicTracerProvider, Span } from '@opentelemetry/sdk-trace-base';

const REPO_ROOT = path.resolve(__dirname, '..', '..', '..');
const SHARED_SH = path.join(REPO_ROOT, 'opt', 'shared.sh');

const MASKED = '****';

const MASKING_ENV = /^(DASH0_MASK_|LUMIGO_SECRET_MASKING_REGEX|LUMIGO_BLACKLIST_REGEX)/;

/** Runs `map_masking_env` from opt/shared.sh and returns the masking-related result. */
const mapMaskingEnv = (input: Record<string, string>): Record<string, string> => {
  const out = execFileSync(
    'bash',
    ['-c', 'source "$1" && map_masking_env && env -0', 'bash', SHARED_SH],
    { env: { PATH: process.env.PATH, ...input }, encoding: 'utf8' }
  );
  return Object.fromEntries(
    out
      .split('\0')
      .filter(Boolean)
      .map((entry) => [entry.slice(0, entry.indexOf('=')), entry.slice(entry.indexOf('=') + 1)])
      .filter(([key]) => MASKING_ENV.test(key))
  );
};

type Captured = {
  'http.request.query': string;
  'http.request.headers': string;
  'http.request.body': string;
  'http.response.headers': string;
  'http.response.body': string;
  'messaging.publish.body': string;
  'messaging.consume.body': string;
};
type Attribute = keyof Captured;

const HTTP_ATTRIBUTES: Attribute[] = [
  'http.request.query',
  'http.request.headers',
  'http.request.body',
  'http.response.headers',
  'http.response.body',
];
const ALL_ATTRIBUTES: Attribute[] = [
  ...HTTP_ATTRIBUTES,
  'messaging.publish.body',
  'messaging.consume.body',
];

// Every context carries the same three kinds of keys: `email` (PII, not masked by default),
// `password` (masked by default), and `orderId` (never masked).
const fixtures = {
  query: { email: 'jane@example.com', password: 'q-pw', orderId: '42' },
  requestHeaders: {
    'content-type': 'application/json',
    'x-email': 'jane@example.com',
    'x-password': 'h-pw',
    authorization: 'Bearer abc',
    'x-signature': 'sig',
    'x-orderid': '42',
  },
  requestBody: { email: 'jane@example.com', password: 'b-pw', apiToken: 't', orderId: '42' },
  responseHeaders: {
    'content-type': 'application/json',
    'x-email': 'jane@example.com',
    'x-password': 'h-pw',
    'x-orderid': '42',
  },
  responseBody: { email: 'jane@example.com', password: 'r-pw', sessionToken: 's', orderId: '42' },
  sqsPublish: {
    QueueUrl: 'https://sqs.eu-central-1.amazonaws.com/123456789012/orders',
    MessageBody: 'order 42 for jane@example.com',
    MessageAttributes: {
      email: { DataType: 'String', StringValue: 'jane@example.com' },
      password: { DataType: 'String', StringValue: 'm-pw' },
      orderId: { DataType: 'String', StringValue: '42' },
    },
  },
  sqsConsume: {
    Messages: [
      {
        MessageId: 'm1',
        Body: 'order 42 for jane@example.com',
        MessageAttributes: {
          email: { DataType: 'String', StringValue: 'jane@example.com' },
          password: { DataType: 'String', StringValue: 'm-pw' },
          orderId: { DataType: 'String', StringValue: '42' },
        },
      },
    ],
  },
};

const span = (attributes: Record<string, string> = {}): Span =>
  new BasicTracerProvider().getTracer('masking').startSpan('s', { attributes }) as Span;

/**
 * Applies the mapped environment, loads the distro in a fresh module registry so
 * node-core builds its scrubbers from that environment, and captures every attribute.
 */
const captureWith = (input: Record<string, string>): Captured => {
  const mapped = mapMaskingEnv(input);
  const saved = { ...process.env };
  for (const key of Object.keys(process.env)) if (MASKING_ENV.test(key)) delete process.env[key];
  Object.assign(process.env, mapped);

  try {
    let captured!: Captured;
    jest.isolateModules(() => {
      const { ExpressHooks } = require('../distro/src/instrumentations/express/express');
      const { preRequestHook, responseHook } = require('../distro/src/instrumentations/aws-sdk/hooks');

      const httpSpan = span();
      const res: any = {
        end: () => undefined,
        send: () => undefined,
        getHeaders: () => fixtures.responseHeaders,
      };
      ExpressHooks.requestHook(httpSpan, {
        req: { query: fixtures.query, headers: fixtures.requestHeaders, body: fixtures.requestBody },
        res,
      });
      res.send(JSON.stringify(fixtures.responseBody));
      res.end();

      const sqsAttributes = { 'rpc.service': 'SQS', 'messaging.destination.name': 'orders' };
      const publishSpan = span({ ...sqsAttributes, 'rpc.method': 'SendMessage' });
      preRequestHook(publishSpan, { request: { commandInput: fixtures.sqsPublish } });
      const consumeSpan = span({ ...sqsAttributes, 'rpc.method': 'ReceiveMessage' });
      responseHook(consumeSpan, {
        response: { request: { commandInput: {} }, requestId: 'r', data: fixtures.sqsConsume },
        moduleVersion: 'x',
      });

      captured = {
        ...(Object.fromEntries(HTTP_ATTRIBUTES.map((a) => [a, httpSpan.attributes[a]])) as any),
        'messaging.publish.body': publishSpan.attributes['messaging.publish.body'],
        'messaging.consume.body': consumeSpan.attributes['messaging.consume.body'],
      };
    });
    return captured;
  } finally {
    process.env = saved;
  }
};

/** Values of every key in the captured JSON whose name ends with `suffix` (case-insensitive). */
const valuesOf = (json: string, suffix: string): unknown[] => {
  const found: unknown[] = [];
  const walk = (node: unknown) => {
    if (Array.isArray(node)) node.forEach(walk);
    else if (node && typeof node === 'object')
      for (const [key, value] of Object.entries(node)) {
        if (key.toLowerCase().endsWith(suffix.toLowerCase())) found.push(value);
        else walk(value);
      }
  };
  walk(JSON.parse(json));
  return found;
};

const expectMasked = (captured: Captured, attributes: Attribute[], key: string) => {
  for (const attribute of attributes) {
    const values = valuesOf(captured[attribute], key);
    expect({ attribute, key, values }).toEqual({
      attribute,
      key,
      values: expect.arrayContaining([MASKED]),
    });
    expect({ attribute, key, values }).toEqual({ attribute, key, values: values.map(() => MASKED) });
  }
};

const expectClear = (captured: Captured, attributes: Attribute[], key: string) => {
  for (const attribute of attributes) {
    const values = valuesOf(captured[attribute], key);
    expect({ attribute, key, hasValues: values.length > 0 }).toEqual({ attribute, key, hasValues: true });
    expect({ attribute, key, masked: values.includes(MASKED) }).toEqual({ attribute, key, masked: false });
  }
};

const except = (...excluded: Attribute[]) => ALL_ATTRIBUTES.filter((a) => !excluded.includes(a));

const DEFAULT_RULES = [
  '.*pass.*',
  '.*key.*',
  '.*secret.*',
  '.*credential.*',
  '.*passphrase.*',
  '.*token.*',
  '.*signature.*',
  '.*authorization.*',
];

describe('DASH0_MASK_* env vars → Node distro masking', () => {
  describe('defaults (nothing configured)', () => {
    let captured: Captured;
    beforeAll(() => {
      captured = captureWith({});
    });

    test('mask passwords in every captured attribute', () => {
      expectMasked(captured, ALL_ATTRIBUTES, 'password');
    });

    test('mask tokens, authorization and signature headers', () => {
      expectMasked(captured, ['http.request.body'], 'apiToken');
      expectMasked(captured, ['http.response.body'], 'sessionToken');
      expectMasked(captured, ['http.request.headers'], 'authorization');
      expectMasked(captured, ['http.request.headers'], 'x-signature');
    });

    test('leave email and orderId in clear text', () => {
      expectClear(captured, ALL_ATTRIBUTES, 'email');
      expectClear(captured, ALL_ATTRIBUTES, 'orderId');
    });
  });

  describe('DASH0_MASK_RULES', () => {
    let captured: Captured;
    beforeAll(() => {
      captured = captureWith({ DASH0_MASK_RULES: '[".*email.*"]' });
    });

    test('masks matching keys in every captured attribute', () => {
      expectMasked(captured, ALL_ATTRIBUTES, 'email');
    });

    test('replaces the default rules, like it does in the Rust extension', () => {
      expectClear(captured, ALL_ATTRIBUTES, 'password');
    });

    test('can extend the defaults by listing them', () => {
      const extended = captureWith({ DASH0_MASK_RULES: JSON.stringify([...DEFAULT_RULES, '.*email.*']) });
      expectMasked(extended, ALL_ATTRIBUTES, 'email');
      expectMasked(extended, ALL_ATTRIBUTES, 'password');
      expectClear(extended, ALL_ATTRIBUTES, 'orderId');
    });

    test('masks the whole SQS message body when it names MessageBody', () => {
      const body = captureWith({ DASH0_MASK_RULES: '[".*MessageBody.*"]' })['messaging.publish.body'];
      expect(JSON.parse(body).MessageBody).toBe(MASKED);
    });

    test('falls back to the defaults when it is not valid JSON', () => {
      expectMasked(captureWith({ DASH0_MASK_RULES: 'not json' }), ALL_ATTRIBUTES, 'password');
    });

    test('still applies to a payload whose per-payload var is invalid', () => {
      const captured = captureWith({ DASH0_MASK_RULES: '[".*email.*"]', DASH0_MASK_REQUEST_BODY: 'nope' });
      expectMasked(captured, ['http.request.body', 'messaging.publish.body'], 'email');
    });
  });

  describe.each<[string, Attribute[]]>([
    ['DASH0_MASK_REQUEST_BODY', ['http.request.body', 'messaging.publish.body']],
    ['DASH0_MASK_RESPONSE_BODY', ['http.response.body', 'messaging.consume.body']],
    ['DASH0_MASK_REQUEST_HEADERS', ['http.request.headers']],
    ['DASH0_MASK_RESPONSE_HEADERS', ['http.response.headers']],
    ['DASH0_MASK_QUERY_PARAMS', ['http.request.query']],
  ])('%s', (envVar, scoped) => {
    test(`masks matching keys in ${scoped.join(', ')} only`, () => {
      const captured = captureWith({ [envVar]: '[".*email.*"]' });
      expectMasked(captured, scoped, 'email');
      expectClear(captured, except(...scoped), 'email');
    });

    test('keeps the defaults for every other attribute', () => {
      const captured = captureWith({ [envVar]: '[".*email.*"]' });
      expectMasked(captured, except(...scoped), 'password');
    });

    test('takes precedence over DASH0_MASK_RULES for its own attributes', () => {
      const captured = captureWith({ DASH0_MASK_RULES: '[".*orderId.*"]', [envVar]: '[".*email.*"]' });
      expectMasked(captured, scoped, 'email');
      expectClear(captured, scoped, 'orderId');
      expectMasked(captured, except(...scoped), 'orderId');
    });
  });

  describe('the value "all"', () => {
    test('DASH0_MASK_REQUEST_BODY=all replaces request and SQS publish bodies with ****', () => {
      const captured = captureWith({ DASH0_MASK_REQUEST_BODY: 'all' });
      expect(captured['http.request.body']).toBe(JSON.stringify(MASKED));
      expect(captured['messaging.publish.body']).toBe(JSON.stringify(MASKED));
      expectClear(captured, ['http.response.body', 'messaging.consume.body'], 'email');
    });

    test('DASH0_MASK_RESPONSE_BODY=all replaces response and SQS consume bodies with ****', () => {
      const captured = captureWith({ DASH0_MASK_RESPONSE_BODY: 'all' });
      expect(captured['http.response.body']).toBe(JSON.stringify(MASKED));
      expect(captured['messaging.consume.body']).toBe(JSON.stringify(MASKED));
    });
  });

  describe('legacy LUMIGO_* variables', () => {
    test('still work on their own', () => {
      const captured = captureWith({
        LUMIGO_SECRET_MASKING_REGEX_HTTP_REQUEST_BODIES: '[".*email.*"]',
      });
      expectMasked(captured, ['http.request.body', 'messaging.publish.body'], 'email');
    });

    test('LUMIGO_BLACKLIST_REGEX still works on its own', () => {
      expectMasked(captureWith({ LUMIGO_BLACKLIST_REGEX: '[".*email.*"]' }), ALL_ATTRIBUTES, 'email');
    });

    test('are overridden by the DASH0_MASK_* equivalent', () => {
      const captured = captureWith({
        LUMIGO_SECRET_MASKING_REGEX: '[".*orderId.*"]',
        DASH0_MASK_RULES: '[".*email.*"]',
        LUMIGO_SECRET_MASKING_REGEX_HTTP_REQUEST_BODIES: '[".*orderId.*"]',
        DASH0_MASK_REQUEST_BODY: '[".*email.*"]',
      });
      expectMasked(captured, ALL_ATTRIBUTES, 'email');
      expectClear(captured, ALL_ATTRIBUTES, 'orderId');
    });
  });
});
