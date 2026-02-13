import { diag, DiagLogLevel, DiagConsoleLogger } from '@opentelemetry/api';
import { DASH0_LOGGING_NAMESPACE } from './constants';

declare global {
  // eslint-disable-next-line @typescript-eslint/no-namespace
  namespace NodeJS {
    interface ProcessEnv {
      DASH0_DEBUG?: string;
    }
  }
}

export const logger = diag.createComponentLogger({
  namespace: DASH0_LOGGING_NAMESPACE,
});

diag.setLogger(new DiagConsoleLogger(), {
  logLevel:
    process.env.DASH0_DEBUG?.toLowerCase() === 'true' ? DiagLogLevel.DEBUG : DiagLogLevel.INFO,
  suppressOverrideMessage: true, // Suppress noise in logs
});
