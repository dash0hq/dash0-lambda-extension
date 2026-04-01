console.log(`[import] top file`);

import { registerInstrumentations } from '@opentelemetry/instrumentation';
import type { Resource } from '@opentelemetry/resources';
import {
  detectResources,
  envDetector,
  processDetector,
  resourceFromAttributes,
  defaultResource,
} from '@opentelemetry/resources';
import {BasicTracerProvider, BatchSpanProcessor, SimpleSpanProcessor} from '@opentelemetry/sdk-trace-base';
import { NodeTracerProvider } from '@opentelemetry/sdk-trace-node';

console.log(`[import] after NodeTracerProvider `);


import {
  DEFAULT_DASH0_EXTENSION_ENDPOINT,
} from './constants';
import { FileSpanExporter } from './exporters';

import Dash0HttpInstrumentation from './instrumentations/https/HttpInstrumentation';
import Dash0UndiciInstrumentation from "./instrumentations/undici/UndiciInstrumentation";

console.log(`[import] after Dash0AwsSdkV3LibInstrumentation `);


import { CompositePropagator, W3CBaggagePropagator } from '@opentelemetry/core';
import { Dash0W3CTraceContextPropagator } from './propagator/w3cTraceContextPropagator';
import { getSpanAttributeMaxLength } from './utils';
import { safeRequire } from './requireUtils';
import { AWSXRayLambdaPropagator } from '@opentelemetry/propagator-aws-xray-lambda';

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
  readonly tracerProvider: BasicTracerProvider;
  readonly resource: Resource;
  readonly instrumentedModules: string[];
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

    const instrumentationsToInstall = [
      new Dash0HttpInstrumentation(...ignoredHostnames),
      new Dash0UndiciInstrumentation(),
    ].filter((i) => i.isApplicable());

    logger.info(`[init] instrumentations created (${instrumentationsToInstall.length}): ${(performance.now() - _t0).toFixed(1)}ms`);

    /*
     * Register instrumentation globally, so that all tracer providers
     * will receive traces. This may be necessary when there is already
     * built-in instrumentation in the app.
     */
    registerInstrumentations({
      instrumentations: instrumentationsToInstall.map((i) => i.getInstrumentation()),
    });

    const instrumentedModules: string[] = instrumentationsToInstall.map((i) =>
      i.getInstrumentedModule()
    );

    logger.info(`[init] instrumentations registered: ${(performance.now() - _t0).toFixed(1)}ms`);
    logger.debug(`Instrumented modules: ${instrumentedModules.join(', ')}`);

    const dashToken = process.env.DASH0_TOKEN;

    if (!dashToken) {
      logger.warn(
        'The Dash0 token is not available (the "DASH0_TOKEN" environment variable is not set): no telemetry will be sent.'
      );
    }

    const infrastructureDetectors = [
      envDetector,
      processDetector,
    ];

    /*
     * These are the resources describing the infrastructure and the runtime that will be
     * sent along with the dependency reporting.
     */
    const infrastructureResource = defaultResource().merge(
      detectResources({
        detectors: infrastructureDetectors,
      })
    );

    const resource = defaultResource()
      .merge(infrastructureResource)

    logger.info(`[init] resource detection: ${(performance.now() - _t0).toFixed(1)}ms`);

    // Build span processors array
    const spanProcessors = [];
    if (process.env.DASH0_DEBUG_SPANDUMP) {
      spanProcessors.push(
        new SimpleSpanProcessor(new FileSpanExporter(process.env.DASH0_DEBUG_SPANDUMP))
      );
    }

    if (dashToken) {

      spanProcessors.push(
        new BatchSpanProcessor(new FileSpanExporter("/tmp/exported"), {
          // The maximum queue size. After the size is reached spans are dropped.
          maxQueueSize: 1000,
          // The maximum batch size of every export. It must be smaller or equal to maxQueueSize.
          maxExportBatchSize: 100,
        })
      );
    }

    // Create providers with processors
    const tracerProvider = new NodeTracerProvider({
      resource,
      spanLimits: {
        attributeValueLengthLimit: getSpanAttributeMaxLength(),
      },
      spanProcessors,
    });

    logger.info(`[init] tracer provider created: ${(performance.now() - _t0).toFixed(1)}ms`);

    tracerProvider.register({
      propagator: new CompositePropagator({
        propagators: [
          new Dash0W3CTraceContextPropagator(),
          new W3CBaggagePropagator(),
          new AWSXRayLambdaPropagator(),
        ],
      }),
    });

    logger.info(`[init] provider registered: ${(performance.now() - _t0).toFixed(1)}ms`);

    logger.info(
      `Dash0 OpenTelemetry Distro started in ${(performance.now() - _t0).toFixed(1)}ms`
    );

    return {
      tracerProvider,
      resource,
      instrumentedModules,
    };
  } catch (err) {
    reportInitError(err);
    throw err;
  }
};
