import type { InstrumentationNodeModuleDefinition } from '@opentelemetry/instrumentation';
import { MySQL2Instrumentation } from '@opentelemetry/instrumentation-mysql2';
import { logger } from '../../logging';
import { TracingInstrumentor } from '../instrumentor';

const MODULE_NAME = 'mysql2';
const CONNECTION_FILE_NAME = 'mysql2/lib/connection.js';

type PatchFunction = (moduleExports: any, moduleVersion?: string) => any;

/**
 * Upstream reads `format` when it patches the connection prototype, and the wrapper it
 * installs closes over that value for the life of the process. `format` is only populated
 * by the hooks for `mysql2` itself and for `mysql2/promise.js`, and both of those run
 * after `mysql2/lib/connection.js` has been patched, because `mysql2/index.js` requires
 * the connection before it assigns `exports.format`. The wrapper is therefore left with
 * `format === undefined`, `getQueryText` falls back to the raw statement, and the query
 * parameter values are never interpolated into `db.statement`.
 *
 * Which hook eventually supplies `format` depends on how the application loads the driver:
 *
 *   require('mysql2')          connection.js, then the mysql2 module hook
 *   require('mysql2/promise')  connection.js, then the promise.js file hook
 *   import 'mysql2'            connection.js, then the mysql2 module hook
 *   import 'mysql2/promise'    connection.js, then the promise.js file hook
 *
 * The module hook does not run at all for the promise entry points, so re-patching from
 * that hook alone would leave them unfixed. Every hook other than the connection one is
 * treated as a possible source of `format`, and the connection prototype is patched again
 * after it runs. Upstream unwraps before it wraps, so re-patching replaces the wrapper
 * instead of stacking another one, and it is a no-op once `format` is already in place.
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

      const repatchConnectionAfter = (patchable: { patch?: PatchFunction }) => {
        const originalPatch = patchable.patch;
        if (!originalPatch) {
          return;
        }
        patchable.patch = (moduleExports: unknown, moduleVersion?: string) => {
          const patched = originalPatch(moduleExports, moduleVersion);
          if (connectionExports) {
            patchConnection(connectionExports, moduleVersion);
          }
          return patched;
        };
      };

      repatchConnectionAfter(definition);
      for (const file of definition.files ?? []) {
        if (file !== connectionFile) {
          repatchConnectionAfter(file);
        }
      }
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
