import Dash0RedisInstrumentation from './RedisInstrumentation';

describe('Dash0RedisInstrumentation', () => {
  const oldEnv = Object.assign({}, process.env);
  beforeEach(() => {
    process.env = { ...oldEnv };
  });

  afterEach(() => {
    jest.clearAllMocks();
    process.env = { ...oldEnv };
  });

  let dash0RedisInstrumentation = new Dash0RedisInstrumentation();

  test('disable redis instrumentation', () => {
    // We've pre-installed redis in package.json
    process.env.LUMIGO_DISABLE_REDIS_INSTRUMENTATION = 'true';
    expect(dash0RedisInstrumentation.isApplicable()).toEqual(false);
  });

  test('getInstrumentedModule should return "redis and be applicable"', () => {
    expect(dash0RedisInstrumentation.getInstrumentedModule()).toEqual('redis');
  });
});
