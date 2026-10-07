// Single source of truth for how integration test files are split into
// CI groups. vitest.config.ts builds its `projects` from this list.
// ci.yml's matrix.group list must be kept in sync with these names by hand.
export const groups = [
    {
        name: 'node',
        include: ['**/test-node-*.test.ts', '**/test-manual-node.test.ts', '**/test-commonjs-bundle.test.ts'],
        // test-node-single-traced.test.ts belongs to the `tracing` group instead — see below.
        exclude: ['**/test-node-single-traced.test.ts'],
    },
    {
        name: 'python',
        include: ['**/test-python-*.test.ts'],
    },
    {
        name: 'java',
        include: ['**/test-java-*.test.ts'],
    },
    {
        name: 'db',
        include: ['**/test-db.test.ts'],
    },
    {
        name: 'tracing',
        include: ['**/test-tracing-scenarios-*.test.ts', '**/test-node-single-traced.test.ts'],
    },
    {
        name: 'sls-plugin',
        include: ['**/test-serverless-plugin.test.ts'],
    },
    {
        name: 'other',
        include: [
            '**/test-00-retries.test.ts',
            '**/test-500-error.test.ts',
            '**/test-payload-truncation.test.ts',
            '**/test-dockerized-lambda.test.ts',
        ],
    },
];
