import * as lambda from 'aws-cdk-lib/aws-lambda';
import { PYTHON_RUNTIMES, NODE_RUNTIMES, JAVA_RUNTIMES } from '../../runtimes';

const RUNTIME_MAP: Record<string, lambda.Runtime> = {
  'python3-10': lambda.Runtime.PYTHON_3_10,
  'python3-11': lambda.Runtime.PYTHON_3_11,
  'python3-12': lambda.Runtime.PYTHON_3_12,
  'python3-13': lambda.Runtime.PYTHON_3_13,
  'python3-14': lambda.Runtime.PYTHON_3_14,
  'nodejs20-x': lambda.Runtime.NODEJS_20_X,
  'nodejs22-x': lambda.Runtime.NODEJS_22_X,
  'nodejs24-x': lambda.Runtime.NODEJS_24_X,
  // aws-cdk-lib 2.231 has no NODEJS_26_X constant yet (nodejs26.x is in public preview), and
  // there is no public.ecr.aws/sam/build-nodejs26.x image, so NodejsFunction bundles with the 24.x one.
  'nodejs26-x': new lambda.Runtime('nodejs26.x', lambda.RuntimeFamily.NODEJS, {
    supportsInlineCode: true,
    bundlingDockerImage: 'public.ecr.aws/sam/build-nodejs24.x',
  }),
  'java17': lambda.Runtime.JAVA_17,
  'java21': lambda.Runtime.JAVA_21,
  'java25': lambda.Runtime.JAVA_25,
};

export function toLambdaRuntime(name: string): lambda.Runtime {
  const rt = RUNTIME_MAP[name];
  if (!rt) throw new Error(`Unknown runtime: ${name}. Add it to RUNTIME_MAP in runtime-utils.ts and to runtimes.ts`);
  return rt;
}

export function toLambdaRuntimes(names: readonly string[]): lambda.Runtime[] {
  return names.map(toLambdaRuntime);
}

export const PYTHON_CDK_RUNTIMES = toLambdaRuntimes(PYTHON_RUNTIMES);
export const NODE_CDK_RUNTIMES = toLambdaRuntimes(NODE_RUNTIMES);
export const JAVA_CDK_RUNTIMES = toLambdaRuntimes(JAVA_RUNTIMES);
