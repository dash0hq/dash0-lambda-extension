import Dash0PgInstrumentation from './PgInstrumentation';

describe('Dash0PgInstrumentation', () => {
  const oldEnv = Object.assign({}, process.env);
  beforeEach(() => {
    process.env = { ...oldEnv };
  });

  afterEach(() => {
    jest.clearAllMocks();
    process.env = { ...oldEnv };
  });

  let dash0PgInstrumentation = new Dash0PgInstrumentation();

  test('getInstrumentedModules should return ["pg"]', () => {
    expect(dash0PgInstrumentation.getInstrumentedModules()).toEqual(['pg']);
  });

});
