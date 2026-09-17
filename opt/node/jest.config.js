module.exports = {
  transform: {
    '^.+\\.tsx?$': ['ts-jest', { tsconfig: 'distro/tsconfig.test.json' }],
  },
  // Everything under opt/node, not just the distro: `test/` holds out-of-process
  // scenario tests that cannot run inside jest (see test/handler-resolution.test.ts).
  // node_modules is excluded by jest's default testPathIgnorePatterns.
  testMatch: ['**/*.test.ts'],
  moduleFileExtensions: ['ts', 'tsx', 'js', 'jsx', 'json', 'node'],
};
