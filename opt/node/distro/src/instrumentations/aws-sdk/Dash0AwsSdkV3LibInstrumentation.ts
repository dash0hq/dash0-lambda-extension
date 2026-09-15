import { Dash0AwsSdkLibInstrumentation } from './Dash0AwsSdkLibInstrumentation';

export class Dash0AwsSdkV3LibInstrumentation extends Dash0AwsSdkLibInstrumentation {
  /**
   * AwsInstrumentation patches the smithy client and middleware stack that every
   * `@aws-sdk/client-*` package is built on, so it applies whenever the function
   * calls any AWS service. The `@smithy` scope covers `@aws-sdk/*` 3.363.0 and
   * later; before that each SDK carried its own copy under the `@aws-sdk` scope.
   */
  getInstrumentedModules(): string[] {
    return ['@smithy/smithy-client', '@aws-sdk/smithy-client'];
  }
}
