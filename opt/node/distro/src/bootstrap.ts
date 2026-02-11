import { OTLPTraceExporter } from '@opentelemetry/exporter-trace-otlp-http';
import { registerInstrumentations } from '@opentelemetry/instrumentation';
import type { Resource } from '@opentelemetry/resources';
import {
  detectResources,
  envDetector,
  processDetector,
  resourceFromAttributes,
  defaultResource,
} from '@opentelemetry/resources';
import { BasicTracerProvider, SimpleSpanProcessor } from '@opentelemetry/sdk-trace-base';
import { NodeTracerProvider } from '@opentelemetry/sdk-trace-node';

import {
  DEFAULT_LUMIGO_TRACES_ENDPOINT,
  TRACING_ENABLED,
} from './constants';
import { FileSpanExporter } from './exporters';

import LumigoGrpcInstrumentation from './instrumentations/@grpc/grpc-js/GrpcInstrumentation';
import LumigoNestInstrumentation from './instrumentations/@nestjs/core/NestInstrumentation';
import LumigoAmqplibInstrumentation from './instrumentations/amqplib/AmqplibInstrumentation';
import LumigoExpressInstrumentation from './instrumentations/express/ExpressInstrumentation';
import LumigoFastifyInstrumentation from './instrumentations/fastify/FastifyInstrumentation';
import LumigoHttpInstrumentation from './instrumentations/https/HttpInstrumentation';
import LumigoIORedisInstrumentation from './instrumentations/ioredis/IORedisInstrumentation';
import LumigoKafkaJsInstrumentation from './instrumentations/kafkajs/KafkaJsInstrumentation';
import LumigoMongoDBInstrumentation from './instrumentations/mongodb/MongoDBInstrumentation';
import LumigoPgInstrumentation from './instrumentations/pg/PgInstrumentation';
import LumigoPrismaInstrumentation from './instrumentations/prisma/PrismaInstrumentation';
import LumigoRedisInstrumentation from './instrumentations/redis/RedisInstrumentation';
import { LumigoAwsSdkV3LibInstrumentation } from './instrumentations/aws-sdk';

import { LumigoW3CTraceContextPropagator } from './propagator/w3cTraceContextPropagator';
import { getSpanAttributeMaxLength } from './utils';
import { safeRequire } from './requireUtils';

declare global {
  // eslint-disable-next-line @typescript-eslint/no-namespace
  namespace NodeJS {
    interface ProcessEnv {
      LUMIGO_DEBUG?: string;
      LUMIGO_DEBUG_SPANDUMP?: string;
      LUMIGO_ENDPOINT?: string;
      LUMIGO_SWITCH_OFF?: string;
      LUMIGO_TRACER_TOKEN?: string;
    }
  }
}

export interface LumigoSdkInitialization {
  readonly tracerProvider: BasicTracerProvider;
  readonly resource: Resource;
  readonly instrumentedModules: string[];
}

import { dirname, join } from 'path';
import { logger } from './logging';
import { ProcessEnvironmentDetector } from './resources/detectors/ProcessEnvironmentDetector';
import { LumigoSpanProcessor } from './resources/spanProcessor';
import { getCombinedSampler } from './samplers/combinedSampler';

const lumigoTraceEndpoint = process.env.LUMIGO_ENDPOINT || DEFAULT_LUMIGO_TRACES_ENDPOINT;

let isTraceInitialized = false;

function reportInitError(err: Error) {
  logger.error(
    'An error occurred while initializing the Lumigo OpenTelemetry Distro: no telemetry will be collected and sent to Lumigo.',
    err
  );
}

export const init = async (): Promise<LumigoSdkInitialization> => {
  if (isTraceInitialized) {
    const message =
      'The Dash0 OpenTelemetry Distro is already initialized: additional attempt to initialize has been ignored.';
    logger.debug(message);

    throw new Error(message);
  }

  isTraceInitialized = true;

  try {
    if (process.env.LUMIGO_SWITCH_OFF?.toLowerCase() === 'true') {
      logger.info(
        'The Dash0 OpenTelemetry Distro is switched off (the "LUMIGO_SWITCH_OFF" environment variable is set): no telemetry will be sent to Dash0.'
      );
      return;
    }

    const { version: distroVersion } =
      safeRequire(join(dirname(__dirname), 'package.json')) ||
      safeRequire(join(__dirname, 'package.json')) ||
      {};

    const ignoredHostnames = [new URL(lumigoTraceEndpoint).hostname];

    const instrumentationsToInstall = [
      new LumigoAmqplibInstrumentation(),
      new LumigoExpressInstrumentation(),
      new LumigoGrpcInstrumentation(),
      new LumigoNestInstrumentation(),
      new LumigoFastifyInstrumentation(),
      new LumigoHttpInstrumentation(...ignoredHostnames),
      new LumigoIORedisInstrumentation(),
      new LumigoKafkaJsInstrumentation(),
      new LumigoMongoDBInstrumentation(),
      new LumigoPgInstrumentation(),
      new LumigoPrismaInstrumentation(),
      new LumigoRedisInstrumentation(),
      new LumigoAwsSdkV3LibInstrumentation(),
    ].filter((i) => i.isApplicable());

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

    logger.debug(`Instrumented modules: ${instrumentedModules.join(', ')}`);

    const lumigoToken = process.env.LUMIGO_TRACER_TOKEN;

    if (!lumigoToken) {
      logger.warn(
        'The Lumigo token is not available (the "LUMIGO_TRACER_TOKEN" environment variable is not set): no telemetry will be sent to Lumigo.'
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

    const framework = instrumentedModules.includes('express') ? 'express' : 'node';

    const processEnvDetectedResource = new ProcessEnvironmentDetector().detect();
    const resource = defaultResource()
      .merge(
        resourceFromAttributes({
          framework,
        })
      )
      .merge(infrastructureResource)
      .merge(resourceFromAttributes(processEnvDetectedResource.attributes || {}));

    // Build span processors array
    const spanProcessors = [];
    if (process.env.LUMIGO_DEBUG_SPANDUMP) {
      spanProcessors.push(
        new SimpleSpanProcessor(new FileSpanExporter(process.env.LUMIGO_DEBUG_SPANDUMP))
      );
    }

    if (lumigoToken) {
      const otlpTraceExporter = new OTLPTraceExporter({
        url: lumigoTraceEndpoint,
        headers: {
          Authorization: `LumigoToken ${lumigoToken.trim()}`,
        },
      });

      if (TRACING_ENABLED) {
        spanProcessors.push(
          new LumigoSpanProcessor(otlpTraceExporter, {
            // The maximum queue size. After the size is reached spans are dropped.
            maxQueueSize: 1000,
            // The maximum batch size of every export. It must be smaller or equal to maxQueueSize.
            maxExportBatchSize: 100,
          })
        );
      } else {
        logger.info(
          'Tracing is disabled (the "LUMIGO_ENABLE_TRACES" environment variable is not set to "true"): no traces will be sent to Lumigo.'
        );
      }
    }

    // Create providers with processors
    const tracerProvider = new NodeTracerProvider({
      sampler: getCombinedSampler(),
      resource,
      spanLimits: {
        attributeValueLengthLimit: getSpanAttributeMaxLength(),
      },
      spanProcessors,
    });

    tracerProvider.register({
      propagator: new LumigoW3CTraceContextPropagator(),
    });

    logger.info(
      `Dash0 OpenTelemetry Distro started`
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
