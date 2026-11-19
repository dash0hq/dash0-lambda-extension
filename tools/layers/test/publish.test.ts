import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { describe, it } from 'node:test';
import { LayerError } from '../src/log.ts';
import { PLACEHOLDER_DESCRIPTION, placeholderZip, publishLayer, type Version } from '../src/publish.ts';
import { compatibleRuntimes } from '../src/runtimes.ts';
import { FakeLayerStore, RecordingLog } from './fake-store.ts';

const LAYER = 'dash0-extension-python';
const DESCRIPTION = 'Send traces, metrics, and logs from AWS Lambda to Dash0.';
const ZIP = new Uint8Array([1, 2, 3]);

function publish(store: FakeLayerStore, version: Version, log = new RecordingLog()) {
  return publishLayer(store, { distro: 'python', layer: LAYER, zip: ZIP, description: DESCRIPTION, version }, log);
}

/** The layer's versions, with placeholders marked and private ones in brackets. */
function versions(store: FakeLayerStore): string[] {
  return (store.versions.get(LAYER) ?? []).map(({ version, description, public: isPublic }) => {
    const label = description === PLACEHOLDER_DESCRIPTION ? `${version}(placeholder)` : `${version}`;
    return isPublic ? label : `[${label}]`;
  });
}

describe('publishLayer', () => {
  it('publishes the next version with auto', async () => {
    const store = new FakeLayerStore().seed(LAYER, [33], DESCRIPTION);
    assert.equal(await publish(store, 'auto'), 34);
    assert.deepEqual(versions(store), ['33', '34']);
  });

  it('publishes with auto even when the counter is ahead', async () => {
    const store = new FakeLayerStore().seed(LAYER, [33], DESCRIPTION).advanceCounter(LAYER, 40);
    assert.equal(await publish(store, 'auto'), 41);
  });

  it('publishes version N when the layer is at N-1', async () => {
    const store = new FakeLayerStore().seed(LAYER, [33], DESCRIPTION);
    assert.equal(await publish(store, 34), 34);
    assert.deepEqual(versions(store), ['33', '34']);
  });

  it('declares the tested runtimes of the distro', async () => {
    const store = new FakeLayerStore().seed(LAYER, [33], DESCRIPTION);
    await publish(store, 34);
    assert.deepEqual(store.version(LAYER, 34)?.compatibleRuntimes, compatibleRuntimes('python'));
  });

  it('pads the gap below N with private placeholders', async () => {
    const store = new FakeLayerStore().seed(LAYER, [33], DESCRIPTION);
    assert.equal(await publish(store, 36), 36);
    assert.deepEqual(versions(store), ['33', '[34(placeholder)]', '[35(placeholder)]', '36']);
  });

  it('pads a layer that has no versions yet, as in a new region', async () => {
    const store = new FakeLayerStore();
    assert.equal(await publish(store, 3), 3);
    assert.deepEqual(versions(store), ['[1(placeholder)]', '[2(placeholder)]', '3']);
  });

  it('only makes N public when an earlier attempt published it, with a warning', async () => {
    const store = new FakeLayerStore().seed(LAYER, [33], DESCRIPTION);
    await store.publish(LAYER, { zip: ZIP, description: DESCRIPTION }); // 34, cancelled before it was made public
    const log = new RecordingLog();
    assert.equal(await publish(store, 34, log), 34);
    assert.deepEqual(versions(store), ['33', '34']);
    assert.match(log.warnings.join(), /version 34 is already published/);
  });

  it('is idempotent when N is already public', async () => {
    const store = new FakeLayerStore().seed(LAYER, [33, 34], DESCRIPTION);
    const log = new RecordingLog();
    assert.equal(await publish(store, 34, log), 34);
    assert.deepEqual(versions(store), ['33', '34']);
    assert.ok(log.infos.includes(`${LAYER} version 34 is already public`));
  });

  it('fails when the layer is already above N', async () => {
    const store = new FakeLayerStore().seed(LAYER, [35], DESCRIPTION);
    await assert.rejects(publish(store, 34), (error) => {
      assert.ok(error instanceof LayerError);
      assert.match(error.message, /already at version 35, above 34/);
      return true;
    });
    assert.deepEqual(versions(store), ['35']);
  });

  it('fails when a placeholder holds N', async () => {
    const store = new FakeLayerStore().seed(LAYER, [33], DESCRIPTION).seed(LAYER, [34], PLACEHOLDER_DESCRIPTION);
    await assert.rejects(publish(store, 34), /version 34 exists but is not a published layer/);
  });

  it('leaves the layer private when the counter is ahead and Lambda assigns another version', async () => {
    const store = new FakeLayerStore().seed(LAYER, [33], DESCRIPTION).advanceCounter(LAYER, 34);
    await assert.rejects(publish(store, 34), /published as version 35, expected 34; not making it public/);
    assert.deepEqual(versions(store), ['33', '[35]']);
  });

  it('stops padding when the counter is ahead', async () => {
    const store = new FakeLayerStore().seed(LAYER, [33], DESCRIPTION).advanceCounter(LAYER, 34);
    await assert.rejects(publish(store, 35), /placeholder got version 35, above 34; the version counter is ahead/);
    assert.deepEqual(versions(store), ['33', '[35(placeholder)]']);
  });

  it('leaves the layer private when Lambda reports other runtimes', async () => {
    const store = new FakeLayerStore().seed(LAYER, [33], DESCRIPTION);
    store.runtimesOverride = ['python3.12'];
    await assert.rejects(publish(store, 'auto'), /expected compatible runtimes .*; not making it public/);
    assert.deepEqual(versions(store), ['33', '[34]']);
  });
});

describe('placeholderZip', () => {
  it('is a valid zip archive', () => {
    const path = join(mkdtempSync(join(tmpdir(), 'placeholder-')), 'placeholder.zip');
    writeFileSync(path, placeholderZip());
    const content = execFileSync('unzip', ['-p', path, 'README-placeholder.txt'], { encoding: 'utf8' });
    assert.match(content, /^Placeholder to align Lambda layer version numbering/);
    execFileSync('unzip', ['-tq', path]); // throws if a CRC or size is wrong
  });
});
