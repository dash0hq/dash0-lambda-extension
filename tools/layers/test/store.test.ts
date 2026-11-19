import assert from 'node:assert/strict';
import { afterEach, beforeEach, describe, it } from 'node:test';
import {
  AddLayerVersionPermissionCommand,
  LambdaClient,
  ListLayerVersionsCommand,
  PublishLayerVersionCommand,
  ResourceConflictException,
  ResourceNotFoundException,
} from '@aws-sdk/client-lambda';
import { mockClient } from 'aws-sdk-client-mock';
import { AwsLayerStore } from '../src/store.ts';

const LAYER = 'dash0-extension-node';
const lambda = mockClient(LambdaClient);
const metadata = { $metadata: {} };

describe('AwsLayerStore', () => {
  let store: AwsLayerStore;
  beforeEach(() => {
    lambda.reset();
    store = new AwsLayerStore('eu-west-1');
  });
  afterEach(() => lambda.reset());

  describe('latest', () => {
    it('returns the newest version Lambda lists', async () => {
      lambda.on(ListLayerVersionsCommand, { LayerName: LAYER, MaxItems: 1 }).resolves({
        LayerVersions: [{ Version: 34, Description: 'real' }],
      });
      assert.deepEqual(await store.latest(LAYER), { version: 34, description: 'real' });
    });

    it('returns undefined for a layer without versions', async () => {
      lambda.on(ListLayerVersionsCommand).resolves({ LayerVersions: [] });
      assert.equal(await store.latest(LAYER), undefined);
    });

    it('returns undefined for a layer that does not exist', async () => {
      lambda.on(ListLayerVersionsCommand).rejects(new ResourceNotFoundException({ message: 'not found', ...metadata }));
      assert.equal(await store.latest(LAYER), undefined);
    });

    it('lets other errors through', async () => {
      lambda.on(ListLayerVersionsCommand).rejects(new Error('AccessDenied'));
      await assert.rejects(store.latest(LAYER), /AccessDenied/);
    });
  });

  describe('publish', () => {
    it('publishes the zip with runtimes and both architectures', async () => {
      lambda.on(PublishLayerVersionCommand).resolves({ Version: 35, CompatibleRuntimes: ['nodejs24.x'] });
      const zip = new Uint8Array([1, 2, 3]);
      assert.deepEqual(await store.publish(LAYER, { zip, description: 'real', compatibleRuntimes: ['nodejs24.x'] }), {
        version: 35,
        compatibleRuntimes: ['nodejs24.x'],
      });
      assert.deepEqual(lambda.commandCalls(PublishLayerVersionCommand)[0]?.args[0].input, {
        LayerName: LAYER,
        Description: 'real',
        Content: { ZipFile: zip },
        CompatibleRuntimes: ['nodejs24.x'],
        CompatibleArchitectures: ['x86_64', 'arm64'],
      });
    });

    it('publishes a placeholder without runtimes or architectures', async () => {
      lambda.on(PublishLayerVersionCommand).resolves({ Version: 34 });
      await store.publish(LAYER, { zip: new Uint8Array(), description: 'placeholder' });
      const input = lambda.commandCalls(PublishLayerVersionCommand)[0]?.args[0].input;
      assert.equal(input?.CompatibleRuntimes, undefined);
      assert.equal(input?.CompatibleArchitectures, undefined);
    });
  });

  describe('makePublic', () => {
    it('grants every account GetLayerVersion', async () => {
      lambda.on(AddLayerVersionPermissionCommand).resolves({});
      assert.equal(await store.makePublic(LAYER, 35), true);
      assert.deepEqual(lambda.commandCalls(AddLayerVersionPermissionCommand)[0]?.args[0].input, {
        LayerName: LAYER,
        VersionNumber: 35,
        StatementId: 'public',
        Action: 'lambda:GetLayerVersion',
        Principal: '*',
      });
    });

    it('returns false when the version is already public', async () => {
      lambda.on(AddLayerVersionPermissionCommand).rejects(new ResourceConflictException({ message: 'exists', ...metadata }));
      assert.equal(await store.makePublic(LAYER, 35), false);
    });

    it('lets other errors through', async () => {
      lambda.on(AddLayerVersionPermissionCommand).rejects(new Error('AccessDenied'));
      await assert.rejects(store.makePublic(LAYER, 35), /AccessDenied/);
    });
  });
});
