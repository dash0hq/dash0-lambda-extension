import { AwsInstrumentation } from '@opentelemetry/instrumentation-aws-sdk';
import { TracingInstrumentor } from '../instrumentor';
import { preRequestHook, responseHook } from './hooks';

export abstract class Dash0AwsSdkLibInstrumentation extends TracingInstrumentor<AwsInstrumentation> {
  override isApplicable(): boolean {
    return (
      super.isApplicable()
    );
  }

  getInstrumentation(): AwsInstrumentation {
    return new AwsInstrumentation({
      responseHook,
      preRequestHook,
    });
  }
}
