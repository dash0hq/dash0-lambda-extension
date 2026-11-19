import * as tested from '../../../integration-tests/runtimes.ts';

export const DISTROS = ['node', 'python', 'java', 'manual'] as const;
export type Distro = (typeof DISTROS)[number];

export function isDistro(value: string): value is Distro {
  return (DISTROS as readonly string[]).includes(value);
}

/** The runtime lists of integration-tests/runtimes.ts, in its `nodejs24-x` format. */
export interface TestedRuntimes {
  NODE_RUNTIMES: readonly string[];
  PYTHON_RUNTIMES: readonly string[];
  JAVA_RUNTIMES: readonly string[];
}

// PublishLayerVersion accepts at most 15 compatible runtimes.
const MAX_COMPATIBLE_RUNTIMES = 15;

/**
 * The Lambda runtimes a layer declares as compatible: the tested runtimes of its
 * language, or of every language for the manual layer, which has no language
 * distro and is also listed for custom runtimes.
 */
export function compatibleRuntimes(distro: Distro, runtimes: TestedRuntimes = tested): string[] {
  const families: readonly (readonly string[])[] = {
    node: [runtimes.NODE_RUNTIMES],
    python: [runtimes.PYTHON_RUNTIMES],
    java: [runtimes.JAVA_RUNTIMES],
    manual: [runtimes.NODE_RUNTIMES, runtimes.PYTHON_RUNTIMES, runtimes.JAVA_RUNTIMES],
  }[distro];
  const compatible = families.flat().map((runtime) => runtime.replaceAll('-', '.'));
  if (distro === 'manual') {
    compatible.push('provided.al2', 'provided.al2023');
  }
  const unique = [...new Set(compatible)];
  if (unique.length < 1 || unique.length > MAX_COMPATIBLE_RUNTIMES) {
    throw new Error(
      `${distro}: Lambda requires 1-${MAX_COMPATIBLE_RUNTIMES} compatible runtimes, got ${unique.length}`,
    );
  }
  return unique;
}
