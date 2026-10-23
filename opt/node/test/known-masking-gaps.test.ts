/*
 * Known masking gaps in the Node distro (SIG-695).
 *
 * Each test here reproduces a gap found by the masking audit
 * (https://claude.ai/code/artifact/4b7225a4-2bd2-424b-8034-b4d57da3e112): a recognizable
 * secret (`s3cr3t-<gap id>`) goes into a payload under a key the default patterns match, or
 * under the key the gap is about, and the test asserts the secure outcome, that the secret
 * appears in no exported attribute.
 *
 * Convention:
 * - `knownGap(...)` inverts the result. It passes while the secret still leaks, so CI stays
 *   green, and fails with "gap fixed: promote this test" once it stops leaking. Then turn the
 *   `knownGap` into a plain `test` so the fix stays covered. A failure to capture the
 *   attribute at all is a normal failure, not a fixed gap.
 * - Plain `test`s in this file are audit rows that did not reproduce. They stay as
 *   regression tests.
 *
 * Like masking-env.test.ts, the environment comes from the real `map_masking_env` in
 * opt/shared.sh, with nothing configured (the defaults). The distro's own instrumentations
 * and hooks run against real libraries and local servers. require-in-the-middle does not
 * see jest's module registry, so each instrumentation is applied by calling its own module
 * patch on the module the test loads.
 */

import { execFileSync } from 'child_process';
import * as path from 'path';
import type { AddressInfo } from 'net';

const REPO_ROOT = path.resolve(__dirname, '..', '..', '..');
const SHARED_SH = path.join(REPO_ROOT, 'opt', 'shared.sh');
const MASKING_ENV = /^(DASH0_MASK_|LUMIGO_SECRET_MASKING_REGEX|LUMIGO_BLACKLIST_REGEX)/;

// node-core reads the masking env once, when it is first required, so set it before any require.
const mapped = execFileSync('bash', ['-c', 'source "$1" && map_masking_env && env -0', 'bash', SHARED_SH], {
  env: { PATH: process.env.PATH },
  encoding: 'utf8',
})
  .split('\0')
  .filter((entry) => MASKING_ENV.test(entry))
  .map((entry) => [entry.slice(0, entry.indexOf('=')), entry.slice(entry.indexOf('=') + 1)]);
for (const key of Object.keys(process.env)) if (MASKING_ENV.test(key)) delete process.env[key];
Object.assign(process.env, Object.fromEntries(mapped));

/* eslint-disable @typescript-eslint/no-var-requires */
const http = require('http');
const express = require('express');
const grpc = require('@grpc/grpc-js');
const pg = require('pg');
const { context } = require('@opentelemetry/api');
const { AsyncLocalStorageContextManager } = require('@opentelemetry/context-async-hooks');
const {
  BasicTracerProvider,
  InMemorySpanExporter,
  SimpleSpanProcessor,
} = require('@opentelemetry/sdk-trace-base');
const { getHttpSpanAttributesFromReq } = require('opentelemetry-instrumentation-express/dist/src/utils/attributes');
const { SnsServiceExtension } = require('@opentelemetry/instrumentation-aws-sdk/build/src/services/sns');
const Dash0HttpInstrumentation = require('../distro/src/instrumentations/https/HttpInstrumentation').default;
const Dash0UndiciInstrumentation = require('../distro/src/instrumentations/undici/UndiciInstrumentation').default;
const { ExpressHooks } = require('../distro/src/instrumentations/express/express');
const Dash0GrpcInstrumentation = require('../distro/src/instrumentations/@grpc/grpc-js/GrpcInstrumentation').default;
const Dash0PgInstrumentation = require('../distro/src/instrumentations/pg/PgInstrumentation').default;
const { Dash0MySQL2Instrumentation } = require('../distro/src/instrumentations/mysql2/Mysql2Instrumentation');
const Dash0MongoDBInstrumentation = require('../distro/src/instrumentations/mongodb/MongoDBInstrumentation').default;
const Dash0RedisInstrumentation = require('../distro/src/instrumentations/redis/RedisInstrumentation').default;
const Dash0IORedisInstrumentation = require('../distro/src/instrumentations/ioredis/IORedisInstrumentation').default;
const Dash0KafkaJsInstrumentation = require('../distro/src/instrumentations/kafkajs/KafkaJsInstrumentation').default;
const Dash0AmqplibInstrumentation = require('../distro/src/instrumentations/amqplib/AmqplibInstrumentation').default;
/* eslint-enable @typescript-eslint/no-var-requires */

type Attributes = Record<string, unknown>;

const secret = (gapId: string) => `s3cr3t-${gapId}`;

class GapReproduced extends Error {}

/** Throws GapReproduced if `value` appears in any of `attributes`; fails normally if none was captured. */
const expectNotLeaked = (attributes: Attributes, names: string[], value: string) => {
  const captured = names.filter((name) => attributes[name] !== undefined);
  expect({ captured: names.filter((name) => captured.includes(name)) }).toEqual({
    captured: expect.arrayContaining([expect.anything()]),
  });
  const leaked = captured.filter((name) => String(attributes[name]).includes(value));
  if (leaked.length > 0) {
    throw new GapReproduced(leaked.map((name) => `${name}=${String(attributes[name])}`).join('\n'));
  }
};

/**
 * A known gap: passes while `run` reproduces it (throws GapReproduced), fails once it does not.
 */
const knownGap = (gapId: string, title: string, reason: string, run: () => unknown | Promise<unknown>) =>
  test(`${gapId}: ${title} [known gap: ${reason}]`, async () => {
    try {
      await run();
    } catch (err) {
      if (err instanceof GapReproduced) return;
      throw err;
    }
    throw new Error(`${gapId}: gap fixed: promote this test to a regular test in this file`);
  });

const exporter = new InMemorySpanExporter();
const provider = new BasicTracerProvider({ spanProcessors: [new SimpleSpanProcessor(exporter)] });
const tracer = provider.getTracer('known-masking-gaps');
const contextManager = new AsyncLocalStorageContextManager().enable();
context.setGlobalContextManager(contextManager);

/** Applies an instrumentation to a module the test loaded, the way require-in-the-middle would. */
const instrument = (instrumentation: any, moduleName: string, moduleExports: unknown) => {
  instrumentation.disable();
  instrumentation.setTracerProvider(provider);
  const definition = instrumentation.getModuleDefinitions().find((d: any) => d.name === moduleName);
  definition.patch(moduleExports, definition.moduleVersion);
  return () => definition.unpatch?.(moduleExports, definition.moduleVersion);
};

const finishedSpans = () => exporter.getFinishedSpans() as { name: string; kind: number; attributes: Attributes }[];
const CLIENT = 2;
const SERVER = 1;
const spanOfKind = (kind: number) => {
  const span = finishedSpans().find((s) => s.kind === kind);
  expect(span).toBeDefined();
  return span!.attributes;
};

const listen = async (server: any): Promise<number> => {
  await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', resolve));
  return (server.address() as AddressInfo).port;
};

/** The remote end of every HTTP client test. Tests set `respond` to what it should send back. */
let respond: (res: any) => void;
const remote = http.createServer((req: any, res: any) => {
  req.resume();
  req.on('end', () => respond(res));
});
let remotePort: number;
const unpatchHttp: (() => void)[] = [];

beforeAll(async () => {
  unpatchHttp.push(instrument(new Dash0HttpInstrumentation().getInstrumentation(), 'http', http));
  remotePort = await listen(remote);
});

afterAll(async () => {
  unpatchHttp.forEach((unpatch) => unpatch());
  await new Promise((resolve) => remote.close(resolve));
  contextManager.disable();
});

beforeEach(() => {
  exporter.reset();
  respond = (res) => res.end();
});

/** Sends a request through the instrumented `http` module and waits for the client span. */
const httpRequest = (
  requestPath: string,
  headers: Record<string, string> = {},
  chunks: string[] = []
): Promise<void> =>
  new Promise((resolve, reject) => {
    const req = http.request(
      { host: '127.0.0.1', port: remotePort, path: requestPath, method: chunks.length ? 'POST' : 'GET', headers },
      (res: any) => {
        res.resume();
        res.on('end', () => setImmediate(resolve));
      }
    );
    req.on('error', reject);
    chunks.slice(0, -1).forEach((chunk) => req.write(chunk));
    req.end(chunks[chunks.length - 1]);
  });

const reply = (headers: Record<string, string | string[]>, body = '') => (res: any) => {
  res.writeHead(200, headers);
  res.end(body);
};

describe('known masking gaps (Node distro, default patterns)', () => {
  describe('GAP-NODE-1: query strings leak through the standard URL attributes', () => {
    const query = (id: string) => `password=${secret(id)}&X-Amz-Security-Token=${secret(id)}`;

    knownGap('GAP-NODE-1a', 'http client http.url and http.target carry the raw query', 'DASH0_MASK_QUERY_PARAMS is not applied to URL attributes', async () => {
      await httpRequest(`/objects?${query('GAP-NODE-1a')}`);
      expectNotLeaked(spanOfKind(CLIENT), ['http.url', 'http.target', 'url.full', 'url.query'], secret('GAP-NODE-1a'));
    });

    knownGap('GAP-NODE-1b', 'http server http.url and http.target carry the raw query', 'no redaction on incoming URLs', async () => {
      await httpRequest(`/objects?${query('GAP-NODE-1b')}`);
      expectNotLeaked(spanOfKind(SERVER), ['http.url', 'http.target', 'url.full', 'url.query'], secret('GAP-NODE-1b'));
    });

    knownGap('GAP-NODE-1c', 'fetch (undici) url.full and url.query carry the raw query', 'upstream undici instrumentation redacts nothing', async () => {
      const instrumentation = new Dash0UndiciInstrumentation().getInstrumentation();
      instrumentation.setTracerProvider(provider);
      try {
        await (await fetch(`http://127.0.0.1:${remotePort}/objects?${query('GAP-NODE-1c')}`)).text();
        await new Promise(setImmediate);
      } finally {
        instrumentation.disable();
      }
      const span = finishedSpans().find((s) => s.attributes['url.full'] !== undefined);
      expect(span).toBeDefined();
      expectNotLeaked(span!.attributes, ['url.full', 'url.query'], secret('GAP-NODE-1c'));
    });

    knownGap('GAP-NODE-1d', 'express http.target carries the query that http.request.query masks', 'includeHttpAttributes copies req.originalUrl as is', async () => {
      const attributes = await expressRequest(`/orders?${query('GAP-NODE-1d')}`, {});
      expect(String(attributes['http.request.query'])).not.toContain(secret('GAP-NODE-1d'));
      expectNotLeaked(attributes, ['http.target'], secret('GAP-NODE-1d'));
    });
  });

  describe('GAP-NODE-2: a JSON response longer than the attribute limit is sent unmasked', () => {
    knownGap('GAP-NODE-2', 'http.response.body > 4096 chars keeps the password', 'body is truncated before JSON.parse, so it is scrubbed as a string', async () => {
      respond = reply(
        { 'content-type': 'application/json' },
        JSON.stringify({ password: secret('GAP-NODE-2'), padding: 'x'.repeat(5000) })
      );
      await httpRequest('/big');
      expectNotLeaked(spanOfKind(CLIENT), ['http.response.body'], secret('GAP-NODE-2'));
    });

    test('control: the same response under the limit is masked by key', async () => {
      respond = reply({ 'content-type': 'application/json' }, JSON.stringify({ password: secret('GAP-NODE-2') }));
      await httpRequest('/small');
      expect(spanOfKind(CLIENT)['http.response.body']).toBeDefined();
      expect(String(spanOfKind(CLIENT)['http.response.body'])).not.toContain(secret('GAP-NODE-2'));
    });
  });

  describe('GAP-NODE-3: only the first chunk of a request body is captured', () => {
    knownGap('GAP-NODE-3', 'JSON body written in two chunks keeps the password', 'the first chunk alone is not valid JSON, so it is scrubbed as a string', async () => {
      await httpRequest('/orders', { 'content-type': 'application/json' }, [
        `{"password":"${secret('GAP-NODE-3')}",`,
        '"orderId":"42"}',
      ]);
      expectNotLeaked(spanOfKind(CLIENT), ['http.request.body'], secret('GAP-NODE-3'));
    });
  });

  describe('GAP-NODE-4: bodies that are not JSON are only masked with "all"', () => {
    knownGap('GAP-NODE-4a', 'form-urlencoded request body (SNS/STS/SQS query protocol)', 'non-JSON strings are never masked by key', async () => {
      await httpRequest('/', { 'content-type': 'application/x-www-form-urlencoded' }, [
        `Action=Publish&Message=hello&password=${secret('GAP-NODE-4a')}`,
      ]);
      expectNotLeaked(spanOfKind(CLIENT), ['http.request.body'], secret('GAP-NODE-4a'));
    });

    knownGap('GAP-NODE-4b', 'STS AssumeRole XML response with SecretAccessKey and SessionToken', 'non-JSON strings are never masked by key', async () => {
      respond = reply(
        { 'content-type': 'text/xml' },
        `<AssumeRoleResponse><AssumeRoleResult><Credentials><AccessKeyId>ASIA</AccessKeyId>` +
          `<SecretAccessKey>${secret('GAP-NODE-4b')}</SecretAccessKey>` +
          `<SessionToken>${secret('GAP-NODE-4b')}</SessionToken></Credentials></AssumeRoleResult></AssumeRoleResponse>`
      );
      await httpRequest('/', { 'content-type': 'application/x-www-form-urlencoded' }, ['Action=AssumeRole']);
      expectNotLeaked(spanOfKind(CLIENT), ['http.response.body'], secret('GAP-NODE-4b'));
    });

    knownGap('GAP-NODE-4c', 'text/plain request body', 'non-JSON strings are never masked by key', async () => {
      await httpRequest('/', { 'content-type': 'text/plain' }, [`password=${secret('GAP-NODE-4c')}`]);
      expectNotLeaked(spanOfKind(CLIENT), ['http.request.body'], secret('GAP-NODE-4c'));
    });

    knownGap('GAP-NODE-4d', 'express text body parsed by express.text()', 'non-JSON strings are never masked by key', async () => {
      const attributes = await expressRequest('/orders', { 'content-type': 'text/plain' }, `password=${secret('GAP-NODE-4d')}`);
      expectNotLeaked(attributes, ['http.request.body'], secret('GAP-NODE-4d'));
    });

    const jsonMessage = (id: string) => Buffer.from(JSON.stringify({ orderId: '42', password: secret(id) }));

    knownGap('GAP-NODE-4e', 'kafkajs messaging.produce.body with a JSON message', 'message value is scrubbed as a string', () => {
      const span = tracer.startSpan('produce');
      new Dash0KafkaJsInstrumentation().getInstrumentation().getConfig().producerHook(span, 'orders', { value: jsonMessage('GAP-NODE-4e') });
      expectNotLeaked(span.attributes, ['messaging.produce.body'], secret('GAP-NODE-4e'));
    });

    knownGap('GAP-NODE-4f', 'kafkajs messaging.consume.body with a JSON message', 'message value is scrubbed as a string', () => {
      const span = tracer.startSpan('consume');
      new Dash0KafkaJsInstrumentation().getInstrumentation().getConfig().consumerHook(span, 'orders', { value: jsonMessage('GAP-NODE-4f') });
      expectNotLeaked(span.attributes, ['messaging.consume.body'], secret('GAP-NODE-4f'));
    });

    knownGap('GAP-NODE-4g', 'amqplib messaging.publish.body with a JSON message', 'message content is scrubbed as a string', () => {
      const span = tracer.startSpan('publish');
      new Dash0AmqplibInstrumentation().getInstrumentation().getConfig().publishHook(span, { content: jsonMessage('GAP-NODE-4g') });
      expectNotLeaked(span.attributes, ['messaging.publish.body'], secret('GAP-NODE-4g'));
    });

    knownGap('GAP-NODE-4h', 'amqplib messaging.consume.body with a JSON message', 'message content is scrubbed as a string', () => {
      const span = tracer.startSpan('consume');
      new Dash0AmqplibInstrumentation().getInstrumentation().getConfig().consumeHook(span, { msg: { content: jsonMessage('GAP-NODE-4h') } });
      expectNotLeaked(span.attributes, ['messaging.consume.body'], secret('GAP-NODE-4h'));
    });
  });

  describe('GAP-NODE-5: AWS responses carry secrets under keys the defaults do not match', () => {
    const awsJson = { 'content-type': 'application/x-amz-json-1.1' };

    knownGap('GAP-NODE-5a', 'SSM GetParameter Value', '"Value" matches no default pattern', async () => {
      respond = reply(awsJson, JSON.stringify({ Parameter: { Name: '/db/password', Type: 'SecureString', Value: secret('GAP-NODE-5a') } }));
      await httpRequest('/', awsJson, ['{"Name":"/db/password","WithDecryption":true}']);
      expectNotLeaked(spanOfKind(CLIENT), ['http.response.body'], secret('GAP-NODE-5a'));
    });

    knownGap('GAP-NODE-5b', 'KMS Decrypt Plaintext', '"Plaintext" matches no default pattern', async () => {
      const plaintext = Buffer.from(secret('GAP-NODE-5b')).toString('base64');
      respond = reply(awsJson, JSON.stringify({ KeyId: 'arn:aws:kms:eu-west-1:123456789012:key/k', Plaintext: plaintext }));
      await httpRequest('/', awsJson, ['{"CiphertextBlob":"AQID"}']);
      expectNotLeaked(spanOfKind(CLIENT), ['http.response.body'], plaintext);
    });

    test('control: SecretsManager SecretString is masked by .*secret.*', async () => {
      respond = reply(awsJson, JSON.stringify({ Name: 'db', SecretString: secret('GAP-NODE-5') }));
      await httpRequest('/', awsJson, ['{"SecretId":"db"}']);
      expect(spanOfKind(CLIENT)['http.response.body']).toBeDefined();
      expect(String(spanOfKind(CLIENT)['http.response.body'])).not.toContain(secret('GAP-NODE-5'));
    });
  });

  describe('GAP-NODE-6: cookies are not masked by the defaults', () => {
    knownGap('GAP-NODE-6a', 'http client cookie request header', '"cookie" matches no default pattern', async () => {
      await httpRequest('/', { cookie: `session=${secret('GAP-NODE-6a')}` });
      expectNotLeaked(spanOfKind(CLIENT), ['http.request.headers'], secret('GAP-NODE-6a'));
    });

    knownGap('GAP-NODE-6b', 'http client set-cookie response header', '"set-cookie" matches no default pattern', async () => {
      respond = reply({ 'set-cookie': `session=${secret('GAP-NODE-6b')}; HttpOnly` });
      await httpRequest('/');
      expectNotLeaked(spanOfKind(CLIENT), ['http.response.headers'], secret('GAP-NODE-6b'));
    });

    knownGap('GAP-NODE-6c', 'express cookie request header', '"cookie" matches no default pattern', async () => {
      const attributes = await expressRequest('/orders', { cookie: `session=${secret('GAP-NODE-6c')}` });
      expectNotLeaked(attributes, ['http.request.headers'], secret('GAP-NODE-6c'));
    });
  });

  describe('GAP-NODE-7: Redis statements and replies carry values', () => {
    const redisCases: [string, string, string[]][] = [
      ['GAP-NODE-7a', 'AUTH', ['default', secret('GAP-NODE-7a')]],
      ['GAP-NODE-7b', 'SET', ['user:1:password', secret('GAP-NODE-7b')]],
      ['GAP-NODE-7c', 'HSET', ['user:1', 'apiToken', secret('GAP-NODE-7c')]],
    ];
    for (const [Instrumentation, library] of [
      [Dash0RedisInstrumentation, 'redis'],
      [Dash0IORedisInstrumentation, 'ioredis'],
    ] as const) {
      for (const [gapId, command, args] of redisCases) {
        const id = `${gapId}-${library}`;
        knownGap(id, `${library} db.statement for ${command}`, 'Dash0 serializer sends every argument as one string', () => {
          const serialize = new Instrumentation().getInstrumentation().getConfig().dbStatementSerializer;
          expectNotLeaked({ 'db.statement': serialize(command, args) }, ['db.statement'], secret(gapId));
        });
      }

      knownGap(`GAP-NODE-7d-${library}`, `${library} db.response.body for GET of a secret`, 'replies are scrubbed as strings', () => {
        const span = tracer.startSpan('GET');
        new Instrumentation().getInstrumentation().getConfig().responseHook(span, 'GET', ['user:1:password'], secret('GAP-NODE-7d'));
        expectNotLeaked(span.attributes, ['db.response.body'], secret('GAP-NODE-7d'));
      });
    }
  });

  describe('GAP-NODE-8: database statements and parameters carry literal values', () => {
    /**
     * Runs a pg query on a closed client. pg fails it on the next tick, without any network,
     * and the instrumentation has already recorded the statement on the span by then.
     */
    const pgQuery = async (text: string, values?: unknown[]) => {
      const unpatch = instrument(new Dash0PgInstrumentation().getInstrumentation(), 'pg', pg);
      try {
        const client = new pg.Client({ host: '127.0.0.1', port: 5432, user: 'app', database: 'app' });
        await client.end();
        await expect(client.query(text, values)).rejects.toThrow('Client was closed');
      } finally {
        unpatch();
      }
      const span = finishedSpans().find((s) => s.attributes['db.system'] === 'postgresql' && s.name !== 'pg.connect');
      expect(span).toBeDefined();
      return span!.attributes;
    };

    knownGap('GAP-NODE-8a', 'pg db.postgresql.values', 'enhancedDatabaseReporting sends parameter values unmasked', async () => {
      const attributes = await pgQuery('UPDATE users SET password = $1 WHERE id = $2', [secret('GAP-NODE-8a'), 42]);
      expectNotLeaked(attributes, ['db.postgresql.values'], secret('GAP-NODE-8a'));
    });

    knownGap('GAP-NODE-8b', 'pg db.statement with an inlined literal', 'statements are not sanitized', async () => {
      const attributes = await pgQuery(`UPDATE users SET password = '${secret('GAP-NODE-8b')}' WHERE id = 42`);
      expectNotLeaked(attributes, ['db.statement', 'db.query.text'], secret('GAP-NODE-8b'));
    });

    knownGap('GAP-NODE-8c', 'mysql2 db.statement with interpolated values', 'Dash0 interpolates values and maskStatement is off', () => {
      // A stand-in for mysql2/lib/connection.js, as in Mysql2Instrumentation.test.ts: mysql2 is not a dependency.
      function Connection(this: any) {
        this.config = { host: 'db.example.com', port: 3306, database: 'app', user: 'app' };
      }
      Connection.prototype.query = (...args: any[]) => (args.find((a) => typeof a === 'function')?.(null, []), {});
      Connection.prototype.execute = Connection.prototype.query;
      const format = (sql: string, values: unknown[]) =>
        values.reduce<string>((acc, v) => acc.replace('?', typeof v === 'string' ? `'${v}'` : String(v)), sql);

      const instrumentation = new Dash0MySQL2Instrumentation();
      instrumentation.setTracerProvider(provider);
      const definition = instrumentation.init().find((d: any) => d.name === 'mysql2');
      definition.files.find((f: any) => f.name === 'mysql2/lib/connection.js').patch(Connection);
      definition.patch({ format });

      new (Connection as any)().query('UPDATE users SET password = ? WHERE id = ?', [secret('GAP-NODE-8c'), 42], () => undefined);
      const span = finishedSpans().find((s) => s.attributes['db.system'] === 'mysql');
      expect(span).toBeDefined();
      expectNotLeaked(span!.attributes, ['db.statement', 'db.query.text'], secret('GAP-NODE-8c'));
    });

    knownGap('GAP-NODE-8d', 'mongodb db.statement', 'enhancedDatabaseReporting disables the "?" value replacer', () => {
      // The mongodb driver is not a dependency; this is the method the patched connection calls for every command.
      const instrumentation = new Dash0MongoDBInstrumentation().getInstrumentation();
      instrumentation.disable();
      const command = { insert: 'users', documents: [{ name: 'jane', password: secret('GAP-NODE-8d') }] };
      const attributes = instrumentation._getSpanAttributes('app', 'users', '127.0.0.1', '27017', command, 'insert');
      expectNotLeaked(attributes, ['db.statement', 'db.query.text'], secret('GAP-NODE-8d'));
    });
  });

  describe('GAP-NODE-9: gRPC payloads ignore every masking setting', () => {
    knownGap('GAP-NODE-9', 'rpc.request.payload and rpc.response.payload, client and server', 'payloads are JSON.stringify-ed without scrubbing', async () => {
      const unpatch = instrument(new Dash0GrpcInstrumentation().getInstrumentation(), '@grpc/grpc-js', grpc);
      const json = {
        serialize: (value: unknown) => Buffer.from(JSON.stringify(value)),
        deserialize: (buffer: Buffer) => JSON.parse(buffer.toString()),
      };
      const service = {
        login: {
          path: '/auth.Auth/Login',
          requestStream: false,
          responseStream: false,
          requestSerialize: json.serialize,
          requestDeserialize: json.deserialize,
          responseSerialize: json.serialize,
          responseDeserialize: json.deserialize,
        },
      };
      const server = new grpc.Server();
      server.addService(service, {
        login: (_call: any, callback: any) => callback(null, { sessionToken: secret('GAP-NODE-9') }),
      });
      const port = await new Promise<number>((resolve, reject) =>
        server.bindAsync('127.0.0.1:0', grpc.ServerCredentials.createInsecure(), (err: any, p: number) => (err ? reject(err) : resolve(p)))
      );
      const Client = grpc.makeGenericClientConstructor(service, 'Auth');
      const client = new Client(`127.0.0.1:${port}`, grpc.credentials.createInsecure());
      try {
        await new Promise((resolve, reject) =>
          client.login({ user: 'jane', password: secret('GAP-NODE-9') }, (err: any, res: unknown) => (err ? reject(err) : resolve(res)))
        );
        await new Promise((resolve) => setTimeout(resolve, 50));
      } finally {
        client.close();
        server.forceShutdown();
        unpatch();
      }
      const payloads = ['rpc.request.payload', 'rpc.response.payload'];
      const attributes = Object.fromEntries(
        [['client', CLIENT], ['server', SERVER]].flatMap(([side, kind]) =>
          payloads.map((name) => [`${side} ${name}`, spanOfKind(kind as number)[name]])
        )
      );
      expectNotLeaked(attributes, Object.keys(attributes), secret('GAP-NODE-9'));
    });
  });

  describe('GAP-NODE-10: SNS SMS Publish puts the phone number in attributes', () => {
    knownGap('GAP-NODE-10', 'messaging.destination and messaging.destination.name carry PhoneNumber', 'the recipient phone number is used as the destination', () => {
      const phoneNumber = '+15555550100';
      const { spanAttributes } = new SnsServiceExtension().requestPreSpanHook(
        { commandName: 'Publish', commandInput: { PhoneNumber: phoneNumber, Message: 'code 1234' } },
        {}
      );
      expectNotLeaked(spanAttributes, ['messaging.destination', 'messaging.destination.name'], phoneNumber);
    });
  });
});

/**
 * Sends a request to a real express app and returns the attributes the express instrumentation
 * would put on its span: upstream's HTTP attributes (includeHttpAttributes) plus the Dash0 hook.
 * The instrumentation itself replaces the Layer constructor, which only require-in-the-middle can do.
 */
async function expressRequest(requestPath: string, headers: Record<string, string>, body?: string): Promise<Attributes> {
  const span = tracer.startSpan('express');
  const app = express();
  app.use(express.text());
  app.use((req: any, res: any, next: any) => {
    span.setAttributes(getHttpSpanAttributesFromReq(req));
    ExpressHooks.requestHook(span, { req, res });
    next();
  });
  app.all('*', (_req: any, res: any) => res.send({ ok: true }));
  const server = app.listen(0, '127.0.0.1');
  await new Promise((resolve) => server.once('listening', resolve));
  try {
    await new Promise<void>((resolve, reject) => {
      const req = http.request(
        { host: '127.0.0.1', port: (server.address() as AddressInfo).port, path: requestPath, method: body ? 'POST' : 'GET', headers },
        (res: any) => res.resume().on('end', resolve)
      );
      req.on('error', reject);
      req.end(body);
    });
  } finally {
    await new Promise((resolve) => server.close(resolve));
  }
  return span.attributes;
}
