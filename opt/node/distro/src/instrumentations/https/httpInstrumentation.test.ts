import Dash0HttpInstrumentation from './HttpInstrumentation';

describe('Dash0HttpInstrumentation', () => {
  let dash0HttpInstrumentation = new Dash0HttpInstrumentation();

  test('getInstrumentedModules should return ["http"]', () => {
    expect(dash0HttpInstrumentation.getInstrumentedModules()).toEqual(['http']);
  });
});
