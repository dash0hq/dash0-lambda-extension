#!/usr/bin/env node
import * as cdk from 'aws-cdk-lib/core';
import { BenchmarkStack } from '../lib/benchmark-stack';

const prefix = process.env.RESOURCE_PREFIX ?? '';
const app = new cdk.App();
new BenchmarkStack(app, `${prefix}BenchmarkStack`);
