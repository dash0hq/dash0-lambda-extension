// Canonical runtime version lists for integration tests.
// Used by both CDK stacks (via runtime-utils.ts) and test files.
// Format matches CDK runtime names with dots replaced by dashes.

export const PYTHON_RUNTIMES = ['python3-10', 'python3-11', 'python3-12', 'python3-13', 'python3-14'] as const;
export const NODE_RUNTIMES = ['nodejs20-x', 'nodejs22-x', 'nodejs24-x'] as const;
export const JAVA_RUNTIMES = ['java17', 'java21', 'java25'] as const;
