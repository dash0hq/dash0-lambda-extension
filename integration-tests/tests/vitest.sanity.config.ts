import { defineConfig } from 'vitest/config';

export default defineConfig({
    test: {
        include: ['**/test-sanity.test.ts'],
        globals: true,
        pool: 'threads',
    },
});
