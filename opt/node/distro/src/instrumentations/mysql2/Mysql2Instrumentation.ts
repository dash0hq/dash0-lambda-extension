import { MySQL2Instrumentation } from '@opentelemetry/instrumentation-mysql2';
import { TracingInstrumentor } from '../instrumentor';

export default class Dash0Mysql2Instrumentation extends TracingInstrumentor<MySQL2Instrumentation> {
  override isApplicable(): boolean {
    return (
      super.isApplicable()
    );
  }

  getInstrumentedModule(): string {
    return 'mysql2';
  }

  getInstrumentation(): MySQL2Instrumentation {
    return new MySQL2Instrumentation();
  }
}
