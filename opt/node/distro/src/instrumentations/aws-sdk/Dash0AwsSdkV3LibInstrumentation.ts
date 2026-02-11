import { Dash0AwsSdkLibInstrumentation } from './Dash0AwsSdkLibInstrumentation';

export class Dash0AwsSdkV3LibInstrumentation extends Dash0AwsSdkLibInstrumentation {
  getInstrumentedModule(): string {
    return '@aws-sdk/client-sqs';
  }
}
