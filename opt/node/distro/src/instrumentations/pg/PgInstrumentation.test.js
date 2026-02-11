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

  test('disable pg instrumentation', () => {
    // We've pre-installed pg in package.json
    process.env.LUMIGO_DISABLE_PG_INSTRUMENTATION = 'true';
    expect(dash0PgInstrumentation.isApplicable()).toEqual(false);
  });

  test('getInstrumentedModule should return "pg"', () => {
    expect(dash0PgInstrumentation.getInstrumentedModule()).toEqual('pg');
  });

  // should not be skipped, see https://lumigo.atlassian.net/browse/RD-11195
  test.skip('requireIfAvailable should return required name', () => {
    // We've pre-installed pg in package.json
    // This test is skipped for now
  });
});
