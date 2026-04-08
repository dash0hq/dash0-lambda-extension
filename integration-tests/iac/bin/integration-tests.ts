#!/usr/bin/env node
import * as cdk from 'aws-cdk-lib/core';
import { SharedResourcesStack } from '../lib/shared-resources-stack';
import { PythonStack, NodeStack, JavaStack, ManualStack, DockerizedStack } from '../lib/integration-tests-stack';
import { PythonTracingScenariosStack } from '../lib/python-tracing-scenarios-stack';
import { NodeTracingScenariosStack } from '../lib/node-tracing-scenarios-stack';
import { JavaTracingScenariosStack } from '../lib/java-tracing-scenarios-stack';
import { DbTestingStack } from '../lib/db-testing-stack';
import { SharedDbStack } from '../lib/shared-db-stack';

const prefix = process.env.RESOURCE_PREFIX ?? '';
const app = new cdk.App();

// Manually deployed, long-lived shared resources (not managed by CI)
new SharedResourcesStack(app, 'SharedResourcesStack');

// Per-PR stacks (deployed and destroyed by CI)
new PythonStack(app, `${prefix}PythonStack`);
new NodeStack(app, `${prefix}NodeStack`);
new JavaStack(app, `${prefix}JavaStack`);
new ManualStack(app, `${prefix}ManualStack`);
new DockerizedStack(app, `${prefix}DockerizedStack`);
new PythonTracingScenariosStack(app, `${prefix}PythonTracingScenariosStack`);
new NodeTracingScenariosStack(app, `${prefix}NodeTracingScenariosStack`);
new JavaTracingScenariosStack(app, `${prefix}JavaTracingScenariosStack`);
new DbTestingStack(app, `${prefix}DbTestingStack`);

// Manually deployed shared database infrastructure
new SharedDbStack(app, 'SharedDbStack');
