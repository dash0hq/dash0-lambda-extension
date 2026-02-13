import Dash0IORedisInstrumentation from './IORedisInstrumentation';

describe('Dash0IORedisInstrumentation', () => {
  const oldEnv = Object.assign({}, process.env);

  beforeEach(() => {
    process.env = { ...oldEnv };
  });

  afterEach(() => {
    jest.clearAllMocks();
    process.env = { ...oldEnv };
  });

  let dash0IORedisInstrumentation = new Dash0IORedisInstrumentation();

  test('getInstrumentedModule should return "ioredis"', () => {
    expect(dash0IORedisInstrumentation.getInstrumentedModule()).toEqual('ioredis');
  });
});
