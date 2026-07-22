#!/usr/bin/env node
import * as cdk from 'aws-cdk-lib/core';
import { PlaygroundStack } from '../lib/playground-stack';

const app = new cdk.App();
new PlaygroundStack(app, 'Sup1312WebAdapterPlayground', {
  // Concrete env is required: the public layer ARNs (Dash0 + Web Adapter)
  // embed the deployment region.
  env: {
    account: process.env.CDK_DEFAULT_ACCOUNT,
    region: process.env.CDK_DEFAULT_REGION,
  },
});
