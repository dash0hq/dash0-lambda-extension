import type { RequestOptions } from 'https';
import { HttpInstrumentation } from '@opentelemetry/instrumentation-http';

import { HttpHooks } from './http';
import { TracingInstrumentor } from '../instrumentor';

/*
 * instrumentation-http reads `options.host` and calls .indexOf() / .match() on it without
 * checking that it is a string, and it does so outside of any safeExecuteInTheMiddle. Node
 * resolves the target from `options.hostname` first and ignores `options.host` entirely
 * whenever `hostname` is set, so a non-string `host` is inert for Node but throws inside the
 * instrumentation -- killing the caller's request, and with it the whole invocation.
 *
 * Dropping the unused `host` is therefore behaviour-preserving for the request. Without a
 * usable `hostname` the options are invalid for Node too, so we skip instrumenting the
 * request entirely and let Node report its own error rather than masking it with ours.
 *
 * Can be removed once https://github.com/open-telemetry/opentelemetry-js/issues/6967 is
 * released and picked up here. Harmless to keep until then.
 *
 * @returns whether the options are safe for the instrumentation to inspect
 */
const makeHostSafe = (request: RequestOptions): boolean => {
  if (request.host == null || typeof request.host === 'string') {
    return true;
  }

  if (typeof request.hostname === 'string' && request.hostname.length > 0) {
    delete request.host;
    return true;
  }

  return false;
};

export default class Dash0HttpInstrumentation extends TracingInstrumentor<HttpInstrumentation> {
  private readonly ignoredHostnames: string[];

  constructor(...ignoredHostnames: string[]) {
    super();

    this.ignoredHostnames = (ignoredHostnames || []).concat(
      [process.env.ECS_CONTAINER_METADATA_URI, process.env.ECS_CONTAINER_METADATA_URI_V4]
        .filter(Boolean)
        .map((url) => {
          try {
            return new URL(url).hostname;
          } catch (err) {
            return;
          }
        })
    );
  }

  getInstrumentedModule = () => 'http';

  getInstrumentation = () =>
    new HttpInstrumentation({
      ignoreOutgoingRequestHook: (request: RequestOptions) => {
        if (!makeHostSafe(request)) {
          return true;
        }

        /*
         * Some requests, like towards the ECS Credentials endpoints, do not have the
         * hostname set, but they do have the host
         */
        const requestHostname = request.hostname || request.host;
        const isRequestIgnored =
          this.ignoredHostnames.includes(requestHostname) ||
          // Unroutable addresses, used by metadata and credential services on all clouds
          /169\.254\.\d+\.\d+.*/gm.test(requestHostname);

        return isRequestIgnored;
      },
      requestHook: HttpHooks.requestHook,
      responseHook: HttpHooks.responseHook,
    });
}
