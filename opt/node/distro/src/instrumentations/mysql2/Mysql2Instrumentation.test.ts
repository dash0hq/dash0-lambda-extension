import type { InstrumentationNodeModuleDefinition } from '@opentelemetry/instrumentation';
import {
  BasicTracerProvider,
  InMemorySpanExporter,
  SimpleSpanProcessor,
} from '@opentelemetry/sdk-trace-base';
import Dash0Mysql2Instrumentation, { Dash0MySQL2Instrumentation } from './Mysql2Instrumentation';

const CONNECTION_FILE_NAME = 'mysql2/lib/connection.js';
const PROMISE_FILE_NAME = 'mysql2/promise.js';

/*
 * Stands in for `mysql2/lib/connection.js`. The instrumentation only cares about the
 * prototype and its `query` / `execute` methods.
 */
const createConnectionModule = () => {
  const respond = function (this: unknown, ..._args: unknown[]) {
    const callback = [..._args].reverse().find((arg) => typeof arg === 'function') as
      | ((err: unknown, results: unknown) => void)
      | undefined;
    callback?.(null, [{ solution: 1 }]);
    return {};
  };

  // The instrumentation patches the prototype, so the methods must not be own properties.
  function Connection(this: any) {
    this.config = { host: 'db.example.com', port: 3306, database: 'test_db', user: 'otel' };
  }
  Connection.prototype.query = respond;
  Connection.prototype.execute = respond;

  return Connection as unknown as new () => {
    query: (...args: unknown[]) => unknown;
    execute: (...args: unknown[]) => unknown;
  };
};

/*
 * Stands in for the `mysql2` module exports. Only `format` is read.
 */
const mysql2Module = {
  format: (sql: string, values: unknown[]) =>
    values.reduce<string>(
      (acc, value) => acc.replace('?', typeof value === 'string' ? `'${value}'` : String(value)),
      sql
    ),
};

describe('Dash0Mysql2Instrumentation', () => {
  let exporter: InMemorySpanExporter;
  let provider: BasicTracerProvider;

  beforeEach(() => {
    exporter = new InMemorySpanExporter();
    provider = new BasicTracerProvider({
      spanProcessors: [new SimpleSpanProcessor(exporter)],
    });
  });

  test('getInstrumentedModules returns ["mysql2"]', () => {
    expect(new Dash0Mysql2Instrumentation().getInstrumentedModules()).toEqual(['mysql2']);
  });

  test('getInstrumentation returns the Dash0 subclass', () => {
    expect(new Dash0Mysql2Instrumentation().getInstrumentation()).toBeInstanceOf(
      Dash0MySQL2Instrumentation
    );
  });

  describe('query parameter values', () => {
    /*
     * `mysql2/lib/connection.js` is always patched first, before anything has handed
     * upstream a `format` function. Which hook supplies it afterwards depends on the entry
     * point the application uses, measured against mysql2 3.24.4:
     *
     *   require('mysql2') and import 'mysql2'                  the mysql2 module hook
     *   require('mysql2/promise') and import 'mysql2/promise'   the promise.js file hook
     *
     * The module hook does not run at all for the promise entry points.
     */
    const FORMAT_CARRIERS = {
      'the mysql2 module hook': (definition: InstrumentationNodeModuleDefinition) => definition,
      'the promise.js file hook': (definition: InstrumentationNodeModuleDefinition) =>
        definition.files.find((file) => file.name === PROMISE_FILE_NAME)!,
    };

    const loadModuleInRealWorldOrder = (
      instrumentation: Dash0MySQL2Instrumentation,
      formatCarrier: keyof typeof FORMAT_CARRIERS = 'the mysql2 module hook'
    ) => {
      instrumentation.setTracerProvider(provider);

      // init() is protected, and calling it is how the definitions under test are produced.
      const definitions = (
        instrumentation as unknown as { init: () => InstrumentationNodeModuleDefinition[] }
      ).init();
      const definition = definitions.find((candidate) => candidate.name === 'mysql2');
      const connectionFile = definition?.files?.find((file) => file.name === CONNECTION_FILE_NAME);
      expect(connectionFile?.patch).toBeDefined();

      const carrier = FORMAT_CARRIERS[formatCarrier](definition!);
      expect(carrier?.patch).toBeDefined();

      const Connection = createConnectionModule();
      connectionFile!.patch!(Connection);
      carrier.patch!(mysql2Module);

      return new Connection();
    };

    const dbStatementOf = () => {
      const spans = exporter.getFinishedSpans();
      expect(spans).toHaveLength(1);
      return spans[0].attributes['db.statement'];
    };

    for (const formatCarrier of Object.keys(FORMAT_CARRIERS) as (keyof typeof FORMAT_CARRIERS)[]) {
      test(`are interpolated into db.statement for query() when format arrives via ${formatCarrier}`, () => {
        const connection = loadModuleInRealWorldOrder(
          new Dash0MySQL2Instrumentation(),
          formatCarrier
        );

        (connection as any).query(
          'SELECT * FROM users WHERE id = ? AND email = ?',
          [42, 'alice@example.com'],
          () => {}
        );

        expect(dbStatementOf()).toEqual(
          "SELECT * FROM users WHERE id = 42 AND email = 'alice@example.com'"
        );
      });

      test(`are interpolated into db.statement for execute() when format arrives via ${formatCarrier}`, () => {
        const connection = loadModuleInRealWorldOrder(
          new Dash0MySQL2Instrumentation(),
          formatCarrier
        );

        (connection as any).execute('SELECT * FROM users WHERE id = ?', [42], () => {});

        expect(dbStatementOf()).toEqual('SELECT * FROM users WHERE id = 42');
      });
    }

    test('are left as placeholders without the re-patch, which is the upstream bug', () => {
      const instrumentation = new (class extends Dash0MySQL2Instrumentation {
        // Opt out of the fix to pin down what it is compensating for.
        protected override init(): InstrumentationNodeModuleDefinition[] {
          return (
            MySQL2InstrumentationPrototypeInit as () => InstrumentationNodeModuleDefinition[]
          ).call(this);
        }
      })();

      const connection = loadModuleInRealWorldOrder(instrumentation, 'the promise.js file hook');

      (connection as any).query('SELECT * FROM users WHERE id = ?', [42], () => {});

      expect(dbStatementOf()).toEqual('SELECT * FROM users WHERE id = ?');
    });
  });
});

// The upstream init(), reached past our override.
const MySQL2InstrumentationPrototypeInit = Object.getPrototypeOf(
  Dash0MySQL2Instrumentation.prototype
).init;
