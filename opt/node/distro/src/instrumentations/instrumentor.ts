import type { Instrumentation } from '@opentelemetry/instrumentation';
import { canRequireModule } from '../requireUtils';

abstract class Instrumentor<T extends Instrumentation> {
  /**
   * The modules this instrumentation patches. These are the modules it hooks,
   * which are not always the ones the application imports: an instrumentation
   * that patches a shared transport applies to every client built on it.
   * Listing a caller-facing package instead switches the instrumentation off
   * for applications that reach the same library by another route.
   */
  abstract getInstrumentedModules(): string[];

  abstract getInstrumentation(options?): T;

  isApplicable() {
    return this.getInstrumentedModules().some(canRequireModule);
  }
}

export abstract class TracingInstrumentor<T extends Instrumentation> extends Instrumentor<T> {
  override isApplicable() {
    return super.isApplicable();
  }
}
