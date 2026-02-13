import { PrismaInstrumentation } from '@prisma/instrumentation';
import { TracingInstrumentor } from '../instrumentor';

export default class Dash0PrismaInstrumentation extends TracingInstrumentor<PrismaInstrumentation> {
  getInstrumentedModule(): string {
    return '@prisma/client';
  }

  getInstrumentation(): PrismaInstrumentation {
    return new PrismaInstrumentation();
  }
}
