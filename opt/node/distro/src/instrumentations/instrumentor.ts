import type { Instrumentation } from '@opentelemetry/instrumentation';
import { canRequireModule } from '../requireUtils';

abstract class Instrumentor<T extends Instrumentation> {
  abstract getInstrumentedModule(): string;

  abstract getInstrumentation(options?): T;

  isApplicable() {
    return canRequireModule(this.getInstrumentedModule());
  }
}

export abstract class TracingInstrumentor<T extends Instrumentation> extends Instrumentor<T> {
  override isApplicable() {
    return super.isApplicable();
  }
}
