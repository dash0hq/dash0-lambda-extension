import { TracingInstrumentor } from '../instrumentor';
import {UndiciInstrumentation} from "@opentelemetry/instrumentation-undici";

export default class Dash0UndiciInstrumentation extends TracingInstrumentor<UndiciInstrumentation> {
  override isApplicable(): boolean {
    return true;
  }

  getInstrumentedModule(): string {
    return 'fetch';
  }

  getInstrumentation(): UndiciInstrumentation {
    return new UndiciInstrumentation();
  }
}
