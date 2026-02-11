import Dash0HttpInstrumentation from './HttpInstrumentation';

describe('Dash0HttpInstrumentation', () => {
  let dash0HttpInstrumentation = new Dash0HttpInstrumentation();

  test('getInstrumentedModule should return "http"', () => {
    expect(dash0HttpInstrumentation.getInstrumentedModule()).toEqual('http');
  });
});
