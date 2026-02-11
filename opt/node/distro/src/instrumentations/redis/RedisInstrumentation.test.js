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

  test('getInstrumentedModule should return "redis and be applicable"', () => {
    expect(dash0RedisInstrumentation.getInstrumentedModule()).toEqual('redis');
  });
});
