const RUNTIME_MAP: Record<string, string> = {
  node: 'dash0-extension-node',
  python: 'dash0-extension-python',
  java: 'dash0-extension-java',
};

export function resolveLayerName(runtime: string | undefined): string | null {
  if (!runtime || typeof runtime !== 'string') {
    return null;
  }

  const normalized = runtime.toLowerCase();
  for (const [prefix, layerName] of Object.entries(RUNTIME_MAP)) {
    if (normalized.startsWith(prefix)) {
      return layerName;
    }
  }

  return null;
}
