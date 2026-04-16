#!/usr/bin/env node
import * as cdk from 'aws-cdk-lib/core';
import { IntegrationTestsStack } from '../lib/integration-tests-stack';
import { SanityChecksStack } from '../lib/sanity-checks-stack';
import { SharedDbStack } from '../lib/shared-db-stack';

const prefix = process.env.RESOURCE_PREFIX ?? '';
const app = new cdk.App();
new IntegrationTestsStack(app, `${prefix}IntegrationTestsStack`, {
  /* If you don't specify 'env', this stack will be environment-agnostic.
   * Account/Region-dependent features and context lookups will not work,
   * but a single synthesized template can be deployed anywhere. */

  /* Uncomment the next line to specialize this stack for the AWS Account
   * and Region that are implied by the current CLI configuration. */
  // env: { account: process.env.CDK_DEFAULT_ACCOUNT, region: process.env.CDK_DEFAULT_REGION },

  /* Uncomment the next line if you know exactly what Account and Region you
   * want to deploy the stack to. */
  // env: { account: '123456789012', region: 'us-east-1' },

  /* For more information, see https://docs.aws.amazon.com/cdk/latest/guide/environments.html */
});

if (process.env.SANITY_NODE_LAYER_ARN && process.env.SANITY_PYTHON_LAYER_ARN) {
  new SanityChecksStack(app, 'SanityChecksStack');
}
new SharedDbStack(app, 'SharedDbStack');
