// Publishes the Dash0 Lambda layers, and resolves and verifies the layer version
// a release publishes in every region.
//
//   node tools/layers/src/layers.ts publish <distro> <layer-name> <zip> --description <text> [--version <N|auto>]
//   node tools/layers/src/layers.ts resolve-version <N|auto>
//   node tools/layers/src/layers.ts verify <N>
//
// resolve-version prints the version on stdout. Everything else goes to stderr.
// The regions come from regions.txt at the repository root, or --regions-file.
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { parseArgs } from 'node:util';
import { isDistro, DISTROS } from './runtimes.ts';
import { publishLayer } from './publish.ts';
import { findDrift, latestVersions, parseVersion, readRegions, resolveVersion } from './release.ts';
import { AwsLayerStore } from './store.ts';
import { LayerError, stderrLog as log } from './log.ts';

const DEFAULT_REGIONS_FILE = resolve(import.meta.dirname, '../../../regions.txt');

async function main(argv: string[]): Promise<void> {
  const { positionals, values } = parseArgs({
    args: argv,
    allowPositionals: true,
    options: {
      description: { type: 'string' },
      version: { type: 'string', default: 'auto' },
      'regions-file': { type: 'string', default: DEFAULT_REGIONS_FILE },
    },
  });
  const [command, ...args] = positionals;
  const storeFor = (region: string) => new AwsLayerStore(region);

  switch (command) {
    case 'publish': {
      const [distro, layer, zipPath] = args;
      if (!distro || !layer || !zipPath || values.description === undefined) {
        throw new LayerError('usage: publish <distro> <layer-name> <zip> --description <text> [--version <N|auto>]');
      }
      if (!isDistro(distro)) {
        throw new LayerError(`distro must be one of ${DISTROS.join(', ')}, got '${distro}'`);
      }
      await publishLayer(
        new AwsLayerStore(),
        {
          distro,
          layer,
          zip: readFileSync(zipPath),
          description: values.description,
          version: parseVersion(values.version),
        },
        log,
      );
      return;
    }
    case 'resolve-version': {
      const [requested] = args;
      if (!requested) {
        throw new LayerError('usage: resolve-version <N|auto>');
      }
      const version = parseVersion(requested);
      const latest = await latestVersions(readRegions(values['regions-file']), storeFor);
      for (const { region, layer, version: current } of latest) {
        log.info(`${region} ${layer}: ${current}`);
      }
      const resolved = resolveVersion(version, latest, log);
      log.info(`Releasing version ${resolved}`);
      console.log(resolved);
      return;
    }
    case 'verify': {
      const [expected] = args;
      const version = expected ? parseVersion(expected) : undefined;
      if (typeof version !== 'number') {
        throw new LayerError('usage: verify <N>');
      }
      const drift = findDrift(version, await latestVersions(readRegions(values['regions-file']), storeFor));
      if (drift.length > 0) {
        throw new LayerError(`layer versions have drifted; refusing to tag:\n  ${drift.join('\n  ')}`);
      }
      log.info(`Every layer is at version ${version} in every region`);
      return;
    }
    default:
      throw new LayerError('usage: layers.ts <publish | resolve-version | verify> ...');
  }
}

try {
  await main(process.argv.slice(2));
} catch (error) {
  if (!(error instanceof LayerError)) {
    throw error;
  }
  console.error(`error: ${error.message}`);
  process.exitCode = 1;
}
