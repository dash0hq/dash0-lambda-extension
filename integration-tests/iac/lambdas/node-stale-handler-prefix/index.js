/*
 * Handler for the `stale-handler-prefix-*` functions.
 *
 * The deployment package contains exactly this one file, at its root, while the functions
 * are configured with `handler: '<prefix>/index.handler'` -- a directory that does not
 * exist in the package. See `lib/integration-tests-stack.ts` and
 * `tests/src/test-node-stale-handler-prefix.test.ts` for what that exercises.
 *
 * Two properties of this file are load-bearing, and both are easy to undo by accident:
 *
 *   - It is CommonJS. The runtime reaches it through `require.resolve`, whose extension
 *     list is `.js`, `.json`, `.node` -- an `.mjs` or `.cjs` file is invisible to it.
 *   - There is no `package.json` beside it. A `"type": "module"` there would make this
 *     file ESM and the runtime's `require()` of it would fail.
 */
exports.handler = async function handler(event) {
  console.log('Handler invoked with event:', event);

  return {
    statusCode: 200,
    body: JSON.stringify({ message: 'Success' }),
  };
};
