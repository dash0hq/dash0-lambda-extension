import Dash0ExpressInstrumentation from './ExpressInstrumentation';

describe('Dash0ExpressInstrumentation', () => {
  let dash0ExpressInstrumentation = new Dash0ExpressInstrumentation();

  test('getInstrumentedModules should return ["express"]', () => {
    expect(dash0ExpressInstrumentation.getInstrumentedModules()).toEqual(['express']);
  });
});
