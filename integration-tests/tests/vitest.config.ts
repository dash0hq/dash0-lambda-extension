import { defineConfig } from 'vitest/config';
import { groups } from './vitest.groups';

const sharedTestConfig = {
    exclude: ['**/test-sanity*'],
    globals: true,
    // Allow more in-file tests to run at once (default is 5)
    maxConcurrency: 12,
    pool: 'threads' as const,
    maxWorkers: 12,
};

export default defineConfig({
    test: {
        ...sharedTestConfig,
        // Running `vitest run` with no --project runs every group, same as before the split.
        projects: groups.map(({ name, include, exclude }) => ({
            test: {
                ...sharedTestConfig,
                name,
                include,
                exclude: [...sharedTestConfig.exclude, ...(exclude ?? [])],
            },
        })),
    },
});
