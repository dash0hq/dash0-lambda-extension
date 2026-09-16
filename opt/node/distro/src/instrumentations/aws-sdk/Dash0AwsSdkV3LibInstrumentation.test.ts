const resolvableModules = new Set<string>();

jest.mock('../../requireUtils', () => ({
  canRequireModule: jest.fn((moduleSpecifier: string) => resolvableModules.has(moduleSpecifier)),
}));

import { Dash0AwsSdkV3LibInstrumentation } from './Dash0AwsSdkV3LibInstrumentation';

/** Stand in for a function's resolvable dependency tree. */
function functionDependsOn(...modules: string[]) {
  resolvableModules.clear();
  modules.forEach((m) => resolvableModules.add(m));
}

describe('Dash0AwsSdkV3LibInstrumentation', () => {
  const instrumentation = new Dash0AwsSdkV3LibInstrumentation();

  afterEach(() => {
    resolvableModules.clear();
  });

  // AwsInstrumentation patches the shared smithy client and middleware stack, so
  // it traces every @aws-sdk/client-* the function uses. Gating it on one client
  // silently disabled AWS tracing for every function that did not use that
  // client: a DynamoDB call then arrived as a bare instrumentation-http span
  // named "POST", with no rpc.service naming the service it reached.
  test('applies to a function that uses DynamoDB but not SQS', () => {
    functionDependsOn('@aws-sdk/client-dynamodb', '@smithy/smithy-client', '@smithy/middleware-stack');

    expect(instrumentation.isApplicable()).toBe(true);
  });

  test('still applies to a function that uses SQS', () => {
    functionDependsOn('@aws-sdk/client-sqs', '@smithy/smithy-client', '@smithy/middleware-stack');

    expect(instrumentation.isApplicable()).toBe(true);
  });

  // @aws-sdk/* below 3.363.0 bundles its own smithy client under the @aws-sdk scope.
  test('applies to AWS SDK versions predating the @smithy scope', () => {
    functionDependsOn('@aws-sdk/client-s3', '@aws-sdk/smithy-client');

    expect(instrumentation.isApplicable()).toBe(true);
  });

  test('does not apply to a function that uses no AWS SDK', () => {
    functionDependsOn('pg', 'express');

    expect(instrumentation.isApplicable()).toBe(false);
  });
});
