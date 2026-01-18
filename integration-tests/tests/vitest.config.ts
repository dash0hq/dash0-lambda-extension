import { defineConfig } from 'vitest/config';

export default defineConfig({
    test: {
        include: ['**/*.{test,spec}.{js,mjs,cjs,ts,mts,cts,jsx,tsx}'],
        globals: true,
        // Allow more in-file tests to run at once (default is 5)
        maxConcurrency: 10,
        pool: 'threads',
        maxWorkers: 12,
    },
});
