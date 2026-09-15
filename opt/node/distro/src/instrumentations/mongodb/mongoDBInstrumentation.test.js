import Dash0MongoDBInstrumentation from './MongoDBInstrumentation';

describe('Dash0MongoDBInstrumentation', () => {
  const oldEnv = Object.assign({}, process.env);
  beforeEach(() => {
    process.env = { ...oldEnv };
  });

  afterEach(() => {
    jest.clearAllMocks();
    process.env = { ...oldEnv };
  });

  let dash0MongoDBInstrumentation = new Dash0MongoDBInstrumentation();

  test('getInstrumentedModules should return ["mongodb"]', () => {
    expect(dash0MongoDBInstrumentation.getInstrumentedModules()).toEqual(['mongodb']);
  });
});
