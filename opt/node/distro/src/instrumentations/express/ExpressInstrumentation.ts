import { ExpressHooks } from './express';
import { ExpressInstrumentation } from 'opentelemetry-instrumentation-express';
import { TracingInstrumentor } from '../instrumentor';

export default class Dash0ExpressInstrumentation extends TracingInstrumentor<ExpressInstrumentation> {
  getInstrumentedModule(): string {
    return 'express';
  }

  getInstrumentation(): ExpressInstrumentation {
    return new ExpressInstrumentation({
      requestHook: ExpressHooks.requestHook,
      includeHttpAttributes: true,
    });
  }
}
