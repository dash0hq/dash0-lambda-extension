import type { LayerStore, LayerVersion, PublishRequest, Published } from '../src/store.ts';
import type { Log } from '../src/log.ts';

interface FakeVersion extends LayerVersion {
  public: boolean;
  compatibleRuntimes: string[];
}

/**
 * The Lambda layers of one region, in memory. Like Lambda, every layer has a
 * version counter that only moves forward, and that runs ahead of the listed
 * versions when versions were deleted.
 */
export class FakeLayerStore implements LayerStore {
  readonly versions = new Map<string, FakeVersion[]>();
  private readonly counters = new Map<string, number>();
  /** Set to make publish report other runtimes than requested. */
  runtimesOverride?: string[];

  /** Adds versions to a layer, as already published and public. */
  seed(layer: string, versions: readonly number[], description: string): this {
    for (const version of versions) {
      this.list(layer).push({ version, description, public: true, compatibleRuntimes: [] });
      this.counters.set(layer, version);
    }
    return this;
  }

  /** Moves the counter as if versions up to `to` had been published and deleted. */
  advanceCounter(layer: string, to: number): this {
    this.counters.set(layer, to);
    return this;
  }

  version(layer: string, version: number): FakeVersion | undefined {
    return this.list(layer).find((entry) => entry.version === version);
  }

  async latest(layer: string): Promise<LayerVersion | undefined> {
    const newest = this.list(layer).at(-1);
    return newest && { version: newest.version, description: newest.description };
  }

  async publish(layer: string, request: PublishRequest): Promise<Published> {
    const version = (this.counters.get(layer) ?? 0) + 1;
    this.counters.set(layer, version);
    const compatibleRuntimes = this.runtimesOverride ?? request.compatibleRuntimes ?? [];
    this.list(layer).push({ version, description: request.description, public: false, compatibleRuntimes });
    return { version, compatibleRuntimes };
  }

  async makePublic(layer: string, version: number): Promise<boolean> {
    const entry = this.version(layer, version);
    if (!entry) {
      throw new Error(`${layer} has no version ${version}`);
    }
    const wasPublic = entry.public;
    entry.public = true;
    return !wasPublic;
  }

  private list(layer: string): FakeVersion[] {
    let list = this.versions.get(layer);
    if (!list) {
      list = [];
      this.versions.set(layer, list);
    }
    return list;
  }
}

/** A Log that records what it was given. */
export class RecordingLog implements Log {
  readonly infos: string[] = [];
  readonly warnings: string[] = [];
  info(message: string): void {
    this.infos.push(message);
  }
  warning(message: string): void {
    this.warnings.push(message);
  }
}
