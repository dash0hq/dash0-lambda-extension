import type { InstrumentationNodeModuleDefinition } from '@opentelemetry/instrumentation';
import { MySQL2Instrumentation } from '@opentelemetry/instrumentation-mysql2';
import { logger } from '../../logging';
import { TracingInstrumentor } from '../instrumentor';

const MODULE_NAME = 'mysql2';
const CONNECTION_FILE_NAME = 'mysql2/lib/connection.js';

/**
 * Upstream reads the `mysql2` module's `format` function when it patches the connection
 * prototype, and the wrapper it installs closes over that value for the life of the
 * process. `mysql2/index.js` requires `./lib/connection.js` before it assigns
 * `exports.format`, so the file hook always runs first and the wrapper is left with
 * `format === undefined`. `getQueryText` then falls back to the raw statement and the
 * query parameter values are never interpolated into `db.statement`.
 *
 * Patching the connection prototype a second time, once the module hook has populated
 * `format`, produces a wrapper that closes over the real function. Upstream unwraps
 * before it wraps, so re-patching replaces the wrapper instead of stacking another one.
 */
export class Dash0MySQL2Instrumentation extends MySQL2Instrumentation {
  protected override init(): InstrumentationNodeModuleDefinition[] {
    const definitions = super.init() as InstrumentationNodeModuleDefinition[];

    for (const definition of definitions) {
      if (definition.name !== MODULE_NAME) {
        continue;
      }

      const connectionFile = definition.files?.find((file) => file.name === CONNECTION_FILE_NAME);
      if (!connectionFile?.patch) {
        logger.debug(
          `No patch found for ${CONNECTION_FILE_NAME}, mysql2 spans will not carry query parameter values.`
        );
        continue;
      }

      /*
       * `mysql2` does not expose `./lib/connection.js` in its package exports, so we cannot
       * require it ourselves. Hold on to the exports object the file hook receives.
       */
      const patchConnection = connectionFile.patch;
      let connectionExports: unknown;
      connectionFile.patch = (moduleExports: unknown, moduleVersion?: string) => {
        connectionExports = moduleExports;
        return patchConnection(moduleExports, moduleVersion);
      };

      const patchModule = definition.patch;
      definition.patch = (moduleExports: unknown, moduleVersion?: string) => {
        const patched = patchModule ? patchModule(moduleExports, moduleVersion) : moduleExports;
        if (connectionExports) {
          patchConnection(connectionExports, moduleVersion);
        }
        return patched;
      };
    }

    return definitions;
  }
}

export default class Dash0Mysql2Instrumentation extends TracingInstrumentor<MySQL2Instrumentation> {
  override isApplicable(): boolean {
    return (
      super.isApplicable()
    );
  }

  getInstrumentedModules(): string[] {
    return [MODULE_NAME];
  }

  getInstrumentation(): MySQL2Instrumentation {
    return new Dash0MySQL2Instrumentation();
  }
}
