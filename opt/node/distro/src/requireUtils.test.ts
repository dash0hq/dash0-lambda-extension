import { safeRequire } from './requireUtils';

describe('safeRequire', () => {
  afterEach(() => {
    jest.clearAllMocks();
  });

  test('does not fail but returns undefined for a non-existing module', () => {
    const result = safeRequire('BlaBlaBlaBla');

    expect(result).toBeUndefined();
  });

  test('does not fail but returns undefined when an errors occurs when loading the module', () => {
    jest.doMock('fs', () => {
      throw Error('RandomError');
    });

    const result = safeRequire('fs');

    expect(result).toBeUndefined();
  });
});
