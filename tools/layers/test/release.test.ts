import assert from 'node:assert/strict';
import { mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { describe, it } from 'node:test';
import {
  findDrift,
  LAYERS,
  latestVersions,
  parseVersion,
  readRegions,
  resolveVersion,
  type LatestVersion,
} from '../src/release.ts';
import { FakeLayerStore, RecordingLog } from './fake-store.ts';

/** Every layer at `version` in each region, with exceptions per region and layer. */
function regionsAt(version: number, regions: readonly string[], exceptions: Record<string, number> = {}) {
  const stores = new Map<string, FakeLayerStore>();
  for (const region of regions) {
    const store = new FakeLayerStore();
    for (const layer of LAYERS) {
      const at = exceptions[`${region} ${layer}`] ?? version;
      if (at > 0) {
        store.seed(layer, [at], 'real');
      }
    }
    stores.set(region, store);
  }
  return (region: string) => stores.get(region)!;
}

describe('latestVersions', () => {
  it('reads every layer in every region, 0 for a layer with no versions', async () => {
    const storeFor = regionsAt(33, ['us-east-1', 'il-central-1'], { 'il-central-1 dash0-extension-manual': 0 });
    const latest = await latestVersions(['us-east-1', 'il-central-1'], storeFor);
    assert.equal(latest.length, 2 * LAYERS.length);
    assert.deepEqual(latest.find((entry) => entry.region === 'il-central-1' && entry.layer === 'dash0-extension-manual'), {
      region: 'il-central-1',
      layer: 'dash0-extension-manual',
      version: 0,
    });
  });
});

describe('resolveVersion', () => {
  // As after the drifted v33 release: Node and Java at 34 in some regions.
  const drifted: LatestVersion[] = [
    { region: 'us-east-1', layer: 'dash0-extension-python', version: 33 },
    { region: 'us-east-1', layer: 'dash0-extension-java', version: 34 },
    { region: 'eu-west-1', layer: 'dash0-extension-node', version: 34 },
    { region: 'il-central-1', layer: 'dash0-extension-manual', version: 0 },
  ];

  it('releases the version after the highest one with auto', () => {
    assert.equal(resolveVersion('auto', drifted, new RecordingLog()), 35);
  });

  it('releases a version above the highest one', () => {
    assert.equal(resolveVersion(37, drifted, new RecordingLog()), 37);
  });

  it('re-releases the highest version with a warning', () => {
    const log = new RecordingLog();
    assert.equal(resolveVersion(34, drifted, log), 34);
    assert.match(log.warnings.join(), /Re-releasing version 34/);
  });

  it('refuses a version below the highest one', () => {
    assert.throws(() => resolveVersion(33, drifted, new RecordingLog()), /some layer is already at version 34/);
  });

  it('releases version 1 when no layer exists yet', () => {
    assert.equal(resolveVersion('auto', [], new RecordingLog()), 1);
  });
});

describe('findDrift', () => {
  it('reports every layer that is not at the version', async () => {
    const regions = ['us-east-1', 'eu-west-1'];
    const latest = await latestVersions(regions, regionsAt(35, regions, { 'eu-west-1 dash0-extension-java': 34 }));
    assert.deepEqual(findDrift(35, latest), ['eu-west-1 dash0-extension-java is at version 34, expected 35']);
  });

  it('reports nothing when every layer is at the version', async () => {
    const regions = ['us-east-1', 'eu-west-1'];
    assert.deepEqual(findDrift(35, await latestVersions(regions, regionsAt(35, regions))), []);
  });
});

describe('parseVersion', () => {
  it('accepts auto and positive numbers', () => {
    assert.equal(parseVersion('auto'), 'auto');
    assert.equal(parseVersion('35'), 35);
  });

  for (const value of ['', '0', '-1', '3.5', '035', 'latest']) {
    it(`rejects '${value}'`, () => {
      assert.throws(() => parseVersion(value), /positive number or 'auto'/);
    });
  }
});

describe('readRegions', () => {
  it('reads one region per line, ignoring blank lines and whitespace', () => {
    const path = join(mkdtempSync(join(tmpdir(), 'regions-')), 'regions.txt');
    writeFileSync(path, 'us-east-1\n\n  eu-west-1 \n');
    assert.deepEqual(readRegions(path), ['us-east-1', 'eu-west-1']);
  });

  it('reads the repository regions.txt', () => {
    const regions = readRegions(join(import.meta.dirname, '../../../regions.txt'));
    assert.ok(regions.includes('us-east-1'));
    assert.equal(new Set(regions).size, regions.length, 'regions.txt lists a region twice');
  });
});
