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

  test('disable mongodb instrumentation', () => {
    // We've pre-installed mongodb in package.json
    process.env.LUMIGO_DISABLE_MONGODB_INSTRUMENTATION = 'true';
    expect(dash0MongoDBInstrumentation.isApplicable()).toEqual(false);
  });

  test('getInstrumentedModule should return "mongodb"', () => {
    expect(dash0MongoDBInstrumentation.getInstrumentedModule()).toEqual('mongodb');
  });
});
