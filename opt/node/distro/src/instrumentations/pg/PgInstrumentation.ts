import { PgInstrumentation } from '@opentelemetry/instrumentation-pg';
import { TracingInstrumentor } from '../instrumentor';

export default class Dash0PgInstrumentation extends TracingInstrumentor<PgInstrumentation> {
  override isApplicable(): boolean {
    return (
      super.isApplicable()
    );
  }

  getInstrumentedModule(): string {
    return 'pg';
  }

  getInstrumentation(): PgInstrumentation {
    return new PgInstrumentation({
      enhancedDatabaseReporting: true,
    });
  }
}
