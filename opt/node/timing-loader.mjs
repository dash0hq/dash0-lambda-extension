import { performance } from 'perf_hooks';
import { createRequire } from 'module';
import { dirname, join } from 'path';
import { existsSync, readFileSync } from 'fs';

const require = createRequire(import.meta.url);
const originalLoad = require('module')._load;

console.log('[timing-loader] Initialized');

function getPackageVersion(modulePath) {
  try {
    // Try to resolve the module's main file
    let resolvedPath;
    try {
      resolvedPath = require.resolve(modulePath);
    } catch {
      return null;
    }

    // Walk up directories to find package.json
    let dir = dirname(resolvedPath);
    for (let i = 0; i < 10; i++) {
      const pkgPath = join(dir, 'package.json');
      if (existsSync(pkgPath)) {
        const pkg = JSON.parse(readFileSync(pkgPath, 'utf8'));
        // Make sure it's the right package (not a parent)
        if (pkg.name && modulePath.includes(pkg.name)) {
          return pkg.version;
        }
      }
      const parentDir = dirname(dir);
      if (parentDir === dir) break;
      dir = parentDir;
    }
  } catch {
    return null;
  }
  return null;
}

require('module')._load = function(request, parent, isMain) {
  // Check if already cached BEFORE loading
  let wasCached = false;
  try {
    const resolved = require.resolve(request);
    wasCached = !!require.cache[resolved];
  } catch {}

  const start = performance.now();
  const result = originalLoad.apply(this, arguments);
  const duration = performance.now() - start;

  if (duration > 10) { // Only log if > 10ms
    let versionStr = '';
    let pathStr = '';
    const cacheStatus = wasCached ? '[CACHED!]' : '[new]';

    if (!request.startsWith('.')) {
      const version = getPackageVersion(request);
      versionStr = version ? `@${version}` : '';

      // Get resolved path
      try {
        const resolved = require.resolve(request);
        // Shorten path for readability - show from node_modules
        const nmIndex = resolved.lastIndexOf('node_modules');
        pathStr = nmIndex >= 0 ? resolved.slice(nmIndex) : resolved;
      } catch {}
    }

    console.log(`[timing] ${duration.toFixed(1)}ms ${cacheStatus}\t${request}${versionStr}\t${pathStr}`);
  }
  return result;
};
