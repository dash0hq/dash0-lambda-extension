console.log(`[import] top file`);

import { registerInstrumentations } from '@opentelemetry/instrumentation';
import { NodeTracerProvider } from '@opentelemetry/sdk-trace-node';

console.log(`[import] after NodeTracerProvider `);


import {
  DEFAULT_DASH0_EXTENSION_ENDPOINT,
} from './constants';


console.log(`[import] after Dash0AwsSdkV3LibInstrumentation `);


import { getSpanAttributeMaxLength } from './utils';
import { safeRequire } from './requireUtils';

declare global {
  // eslint-disable-next-line @typescript-eslint/no-namespace
  namespace NodeJS {
    interface ProcessEnv {
      DASH0_DEBUG?: string;
      DASH0_DEBUG_SPANDUMP?: string;
      DASH0_EXTENSION_ENDPOINT?: string;
      DASH0_SWITCH_OFF?: string;
      DASH0_TOKEN?: string;
    }
  }
}

export interface Dash0SdkInitialization {
  readonly tracerProvider: any;
}

import { dirname, join } from 'path';
import { logger } from './logging';

const _t0 = performance.now();
console.log(`[init] module body start: ${_t0.toFixed(1)}ms`);

const traceEndpoint = process.env.DASH0_EXTENSION_ENDPOINT || DEFAULT_DASH0_EXTENSION_ENDPOINT;

let isTraceInitialized = false;

function reportInitError(err: Error) {
  logger.error(
    'An error occurred while initializing the Dash0 OpenTelemetry Distro: no telemetry will be collected and sent.',
    err
  );
}

export const init = async (): Promise<Dash0SdkInitialization> => {
  if (isTraceInitialized) {
    const message =
      'The Dash0 OpenTelemetry Distro is already initialized: additional attempt to initialize has been ignored.';
    logger.debug(message);

    throw new Error(message);
  }

  isTraceInitialized = true;

  try {
    logger.info(`[init] init() called: ${(performance.now() - _t0).toFixed(1)}ms`);

    if (process.env.DASH0_SWITCH_OFF?.toLowerCase() === 'true') {
      logger.info(
        'The Dash0 OpenTelemetry Distro is switched off (the "DASH0_SWITCH_OFF" environment variable is set): no telemetry will be sent to Dash0.'
      );
      return;
    }

    const { version: distroVersion } =
      safeRequire(join(dirname(__dirname), 'package.json')) ||
      safeRequire(join(__dirname, 'package.json')) ||
      {};

    const ignoredHostnames = [new URL(traceEndpoint).hostname];

    logger.info(`[init] setup done: ${(performance.now() - _t0).toFixed(1)}ms`);



    logger.info(`[init] instrumentations registered: ${(performance.now() - _t0).toFixed(1)}ms`);



    logger.info(`[init] resource detection: ${(performance.now() - _t0).toFixed(1)}ms`);


    // Create providers with processors
    const tracerProvider = new NodeTracerProvider({
      spanLimits: {
        attributeValueLengthLimit: getSpanAttributeMaxLength(),
      },
    });

    logger.info(`[init] tracer provider created: ${(performance.now() - _t0).toFixed(1)}ms`);

    tracerProvider.register();

    logger.info(`[init] provider registered: ${(performance.now() - _t0).toFixed(1)}ms`);

    logger.info(
      `Dash0 OpenTelemetry Distro started in ${(performance.now() - _t0).toFixed(1)}ms`
    );

    return {
      tracerProvider,
    };
  } catch (err) {
    reportInitError(err);
    throw err;
  }
};
