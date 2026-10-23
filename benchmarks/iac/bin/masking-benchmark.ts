#!/usr/bin/env node
import * as cdk from 'aws-cdk-lib/core';
import { MaskingBenchmarkStack } from '../lib/masking-benchmark-stack';

const prefix = process.env.RESOURCE_PREFIX ?? '';
const app = new cdk.App();
new MaskingBenchmarkStack(app, `${prefix}MaskingBenchmarkStack`);
