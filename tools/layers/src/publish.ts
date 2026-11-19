import { crc32 } from 'node:zlib';
import { compatibleRuntimes, type Distro } from './runtimes.ts';
import type { LayerStore, LayerVersion } from './store.ts';
import { LayerError, type Log } from './log.ts';

/** A layer version to publish, or `auto` for whatever Lambda assigns next. */
export type Version = number | 'auto';

export const PLACEHOLDER_DESCRIPTION =
  'Placeholder to align layer version numbering across regions - safe to delete';

/**
 * What publishing a layer takes, given its latest version. Lambda assigns layer
 * versions from a per-region counter that can't be set, so a release that must
 * get version N everywhere first fills the gap below N with placeholders.
 */
export type PublishPlan =
  /** Publish now: the version is `auto`, or the layer is at N-1. */
  | { action: 'publish' }
  /** Publish placeholders until the layer is at padTo (N-1), then publish. */
  | { action: 'pad-and-publish'; padTo: number }
  /** An earlier attempt of the same release published N; only make it public. */
  | { action: 'make-public'; version: number }
  | { action: 'fail'; reason: string };

export function planPublish(
  layer: string,
  latest: LayerVersion | undefined,
  version: Version,
  description: string,
): PublishPlan {
  if (version === 'auto') {
    return { action: 'publish' };
  }
  const current = latest?.version ?? 0;
  if (current > version) {
    return { action: 'fail', reason: `${layer} is already at version ${current}, above ${version}` };
  }
  if (current === version) {
    if (latest?.description !== description) {
      return {
        action: 'fail',
        reason: `${layer} version ${version} exists but is not a published layer (description: '${latest?.description}')`,
      };
    }
    return { action: 'make-public', version };
  }
  if (current === version - 1) {
    return { action: 'publish' };
  }
  return { action: 'pad-and-publish', padTo: version - 1 };
}

export interface PublishOptions {
  distro: Distro;
  layer: string;
  zip: Uint8Array;
  description: string;
  version: Version;
}

/**
 * Publishes a layer version and makes it public. With a version N, fails rather
 * than publish anything but N, and leaves a wrong version private. Publishing N
 * again is idempotent: a layer already at N is only made public.
 */
export async function publishLayer(store: LayerStore, options: PublishOptions, log: Log): Promise<number> {
  const { distro, layer, zip, description, version } = options;
  const plan = planPublish(layer, await store.latest(layer), version, description);

  let published: number;
  switch (plan.action) {
    case 'fail':
      throw new LayerError(plan.reason);
    case 'make-public':
      log.warning(`${layer} version ${plan.version} is already published; skipping the publish and only making sure it is public`);
      published = plan.version;
      break;
    case 'pad-and-publish':
      await padTo(store, layer, plan.padTo, log);
    // falls through
    case 'publish': {
      log.info(`Publishing Lambda Extension to layer "${layer}"`);
      const expectedRuntimes = compatibleRuntimes(distro);
      const result = await store.publish(layer, { zip, description, compatibleRuntimes: expectedRuntimes });
      if (!sameMembers(result.compatibleRuntimes, expectedRuntimes)) {
        throw new LayerError(
          `${layer} version ${result.version}: expected compatible runtimes ${expectedRuntimes.join(' ')}, got ${result.compatibleRuntimes.join(' ')}; not making it public`,
        );
      }
      if (version !== 'auto' && result.version !== version) {
        throw new LayerError(
          `${layer} was published as version ${result.version}, expected ${version}; not making it public`,
        );
      }
      published = result.version;
      break;
    }
  }

  log.info(`Making ${layer} version ${published} public`);
  if (!(await store.makePublic(layer, published))) {
    log.info(`${layer} version ${published} is already public`);
  }
  return published;
}

/** Publishes private placeholder versions until the layer is at the target. */
async function padTo(store: LayerStore, layer: string, target: number, log: Log): Promise<void> {
  const zip = placeholderZip();
  for (;;) {
    const { version } = await store.publish(layer, { zip, description: PLACEHOLDER_DESCRIPTION });
    log.info(`Published placeholder ${layer} version ${version}`);
    if (version > target) {
      // Versions published and deleted before moved the counter past the listed ones.
      throw new LayerError(
        `${layer} placeholder got version ${version}, above ${target}; the version counter is ahead, so version ${target + 1} can't be published here`,
      );
    }
    if (version === target) {
      return;
    }
  }
}

function sameMembers(a: readonly string[], b: readonly string[]): boolean {
  return a.length === b.length && [...a].sort().join() === [...b].sort().join();
}

/** A zip archive holding one uncompressed README, as the placeholder layer content. */
export function placeholderZip(): Uint8Array {
  const name = Buffer.from('README-placeholder.txt');
  const data = Buffer.from('Placeholder to align Lambda layer version numbering across regions. Safe to delete.\n');
  const crc = crc32(data);
  const dosDate = 0x21; // 1980-01-01, the earliest date a zip entry can carry

  const local = Buffer.alloc(30);
  local.writeUInt32LE(0x04034b50, 0);
  local.writeUInt16LE(20, 4); // version needed to extract
  local.writeUInt16LE(dosDate, 12);
  local.writeUInt32LE(crc, 14);
  local.writeUInt32LE(data.length, 18); // compressed size (stored)
  local.writeUInt32LE(data.length, 22);
  local.writeUInt16LE(name.length, 26);

  const central = Buffer.alloc(46);
  central.writeUInt32LE(0x02014b50, 0);
  central.writeUInt16LE(20, 4); // version made by
  central.writeUInt16LE(20, 6); // version needed to extract
  central.writeUInt16LE(dosDate, 14);
  central.writeUInt32LE(crc, 16);
  central.writeUInt32LE(data.length, 20);
  central.writeUInt32LE(data.length, 24);
  central.writeUInt16LE(name.length, 28);
  // The local header's offset, at 42, is 0.

  const end = Buffer.alloc(22);
  end.writeUInt32LE(0x06054b50, 0);
  end.writeUInt16LE(1, 8); // entries on this disk
  end.writeUInt16LE(1, 10); // entries in total
  end.writeUInt32LE(central.length + name.length, 12);
  end.writeUInt32LE(local.length + name.length + data.length, 16);

  return Buffer.concat([local, name, data, central, name, end]);
}
