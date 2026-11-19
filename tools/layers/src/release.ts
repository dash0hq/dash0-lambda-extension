import { readFileSync } from 'node:fs';
import { DISTROS } from './runtimes.ts';
import type { LayerStore } from './store.ts';
import type { Version } from './publish.ts';
import { LayerError, type Log } from './log.ts';

/** The layers a release publishes, in every region. */
export const LAYERS = DISTROS.map((distro) => `dash0-extension-${distro}`);

/** The regions in a regions.txt file: one per line, blank lines ignored. */
export function readRegions(path: string): string[] {
  return readFileSync(path, 'utf8')
    .split('\n')
    .map((line) => line.trim())
    .filter((line) => line !== '');
}

export interface LatestVersion {
  region: string;
  layer: string;
  /** 0 when the layer has no versions in the region yet. */
  version: number;
}

/** The latest version of every layer in every region. */
export async function latestVersions(
  regions: readonly string[],
  storeFor: (region: string) => LayerStore,
): Promise<LatestVersion[]> {
  return Promise.all(
    regions.flatMap((region) => {
      const store = storeFor(region);
      return LAYERS.map(async (layer) => ({
        region,
        layer,
        version: (await store.latest(layer))?.version ?? 0,
      }));
    }),
  );
}

/**
 * The version a release publishes in every region:
 *   - auto:               the highest version of any layer in any region, plus one.
 *   - a version above it: that version; publishing pads the counters up to it.
 *   - the highest itself: re-releases it, with a warning; publishing skips the
 *                         layers already at it.
 *   - a version below it: fails, since some layer can no longer get it.
 */
export function resolveVersion(requested: Version, latest: readonly LatestVersion[], log: Log): number {
  const highest = Math.max(0, ...latest.map(({ version }) => version));
  const version = requested === 'auto' ? highest + 1 : requested;
  if (version < highest) {
    throw new LayerError(`cannot release version ${version}: some layer is already at version ${highest}`);
  }
  if (version === highest) {
    log.warning(`Re-releasing version ${version}: layers already at it are skipped`);
  }
  return version;
}

/** The layers that are not at the release version, as error messages. */
export function findDrift(version: number, latest: readonly LatestVersion[]): string[] {
  return latest
    .filter((entry) => entry.version !== version)
    .map(({ region, layer, version: actual }) => `${region} ${layer} is at version ${actual}, expected ${version}`);
}

/** Parses `auto` or a positive layer version. */
export function parseVersion(value: string): Version {
  if (value === 'auto') {
    return 'auto';
  }
  if (!/^[1-9][0-9]*$/.test(value)) {
    throw new LayerError(`version must be a positive number or 'auto', got '${value}'`);
  }
  return Number(value);
}
