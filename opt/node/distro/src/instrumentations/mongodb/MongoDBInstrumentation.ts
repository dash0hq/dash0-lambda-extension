import { MongoDBInstrumentation } from '@opentelemetry/instrumentation-mongodb';
import { TracingInstrumentor } from '../instrumentor';

export default class Dash0MongoDBInstrumentation extends TracingInstrumentor<MongoDBInstrumentation> {
  override isApplicable(): boolean {
    return (
      super.isApplicable()
    );
  }
  getInstrumentedModule(): string {
    return 'mongodb';
  }

  getInstrumentation(): MongoDBInstrumentation {
    return new MongoDBInstrumentation({
      enhancedDatabaseReporting: true,
    });
  }
}
