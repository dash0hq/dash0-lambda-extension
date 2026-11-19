import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import { compatibleRuntimes, DISTROS } from '../src/runtimes.ts';

const lists = (node: string[], python: string[] = [], java: string[] = []) => ({
  NODE_RUNTIMES: node,
  PYTHON_RUNTIMES: python,
  JAVA_RUNTIMES: java,
});

describe('compatibleRuntimes', () => {
  it('converts the tested runtimes of each family to Lambda runtime names', () => {
    const tested = lists(['nodejs24-x', 'nodejs26-x'], ['python3-14'], ['java25']);
    assert.deepEqual(compatibleRuntimes('node', tested), ['nodejs24.x', 'nodejs26.x']);
    assert.deepEqual(compatibleRuntimes('python', tested), ['python3.14']);
    assert.deepEqual(compatibleRuntimes('java', tested), ['java25']);
  });

  it('lists every family and the custom runtimes for the manual layer', () => {
    const tested = lists(['nodejs24-x', 'nodejs26-x'], ['python3-14'], ['java25']);
    assert.deepEqual(compatibleRuntimes('manual', tested), [
      'nodejs24.x',
      'nodejs26.x',
      'python3.14',
      'java25',
      'provided.al2',
      'provided.al2023',
    ]);
  });

  it('accepts up to 15 runtimes, the Lambda limit', () => {
    const node = (count: number) => Array.from({ length: count }, (_, n) => `nodejs${n}-x`);
    assert.equal(compatibleRuntimes('node', lists(node(15))).at(-1), 'nodejs14.x');
    assert.throws(() => compatibleRuntimes('node', lists(node(16))), /got 16/);
  });

  it('counts the custom runtimes towards the limit of the manual layer', () => {
    const node = Array.from({ length: 11 }, (_, n) => `nodejs${n}-x`);
    assert.equal(compatibleRuntimes('manual', lists(node, ['python3-14'], ['java25'])).length, 15);
    assert.throws(() => compatibleRuntimes('manual', lists(node, ['python3-14'], ['java25', 'java29'])), /got 16/);
  });

  it('fails for a family with no tested runtimes', () => {
    assert.throws(() => compatibleRuntimes('node', lists([])), /got 0/);
  });

  it('fits the runtimes of integration-tests/runtimes.ts for every layer', () => {
    for (const distro of DISTROS) {
      assert.ok(compatibleRuntimes(distro).length > 0, distro);
    }
  });
});
