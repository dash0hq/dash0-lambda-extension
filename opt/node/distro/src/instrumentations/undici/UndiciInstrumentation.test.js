import Dash0UndiciInstrumentation from "./UndiciInstrumentation";

describe('Dash0UndiciInstrumentation', () => {
  const oldEnv = Object.assign({}, process.env);

  beforeEach(() => {
    process.env = { ...oldEnv };
  });

  afterEach(() => {
    jest.clearAllMocks();
    process.env = { ...oldEnv };
  });

  let dash0UndiciInstrumentation = new Dash0UndiciInstrumentation();

  test('getInstrumentedModules should return ["fetch"]', () => {
    expect(dash0UndiciInstrumentation.getInstrumentedModules()).toEqual(['fetch']);
  });
});
