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

  test('disable ioredis instrumentation', () => {
    // We've pre-installed ioredis in package.json
    process.env.LUMIGO_DISABLE_IOREDIS_INSTRUMENTATION = 'true';
    expect(dash0IORedisInstrumentation.isApplicable()).toEqual(false);
  });
});
