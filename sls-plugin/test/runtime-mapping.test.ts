import { resolveLayerName } from '../src/runtime-mapping';

describe('resolveLayerName', () => {
  it.each([
    ['nodejs20.x', 'dash0-extension-node'],
    ['nodejs18.x', 'dash0-extension-node'],
    ['nodejs16.x', 'dash0-extension-node'],
    ['python3.12', 'dash0-extension-python'],
    ['python3.9', 'dash0-extension-python'],
    ['java21', 'dash0-extension-java'],
    ['java17', 'dash0-extension-java'],
    ['java11', 'dash0-extension-java'],
  ])('maps %s to %s', (runtime, expected) => {
    expect(resolveLayerName(runtime)).toBe(expected);
  });

  it.each([
    ['ruby3.3'],
    ['dotnet8'],
    ['go1.x'],
    ['provided.al2023'],
    ['provided.al2'],
  ])('returns null for unsupported runtime %s', (runtime) => {
    expect(resolveLayerName(runtime)).toBeNull();
  });

  it('returns null for undefined', () => {
    expect(resolveLayerName(undefined)).toBeNull();
  });

  it('is case-insensitive', () => {
    expect(resolveLayerName('NodeJS20.x')).toBe('dash0-extension-node');
    expect(resolveLayerName('Python3.12')).toBe('dash0-extension-python');
    expect(resolveLayerName('JAVA21')).toBe('dash0-extension-java');
  });
});
