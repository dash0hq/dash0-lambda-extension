import 'jest-json';

import { ProcessEnvironmentDetector } from './ProcessEnvironmentDetector';

describe('ProcessEnvironmentDetector', () => {
  const ORIGINAL_PROCESS_ENV = process.env;

  afterAll(() => {
    process.env = ORIGINAL_PROCESS_ENV;
  });

  beforeEach(() => {
    /*
     * We have a limit on the size of env we sent to the backend, and the env
     * in the CI/CD goes over the limit, so the additional env vars we want to
     * check for scrubbing get dropped.
     */
    process.env = {};
  });
});
