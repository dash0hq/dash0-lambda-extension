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
import {BasicTracerProvider, BatchSpanProcessor, SimpleSpanProcessor} from '@opentelemetry/sdk-trace-base';
import { NodeTracerProvider } from '@opentelemetry/sdk-trace-node';

import {
  DEFAULT_DASH0_EXTENSION_ENDPOINT,
} from './constants';
import { FileSpanExporter } from './exporters';

import Dash0GrpcInstrumentation from './instrumentations/@grpc/grpc-js/GrpcInstrumentation';
import Dash0NestInstrumentation from './instrumentations/@nestjs/core/NestInstrumentation';
import Dash0AmqplibInstrumentation from './instrumentations/amqplib/AmqplibInstrumentation';
import Dash0ExpressInstrumentation from './instrumentations/express/ExpressInstrumentation';
import Dash0FastifyInstrumentation from './instrumentations/fastify/FastifyInstrumentation';
import Dash0HttpInstrumentation from './instrumentations/https/HttpInstrumentation';
import Dash0IORedisInstrumentation from './instrumentations/ioredis/IORedisInstrumentation';
import Dash0KafkaJsInstrumentation from './instrumentations/kafkajs/KafkaJsInstrumentation';
import Dash0MongoDBInstrumentation from './instrumentations/mongodb/MongoDBInstrumentation';
import Dash0Mysql2Instrumentation from './instrumentations/mysql2/Mysql2Instrumentation';
import Dash0PgInstrumentation from './instrumentations/pg/PgInstrumentation';
import Dash0PrismaInstrumentation from './instrumentations/prisma/PrismaInstrumentation';
import Dash0RedisInstrumentation from './instrumentations/redis/RedisInstrumentation';
import Dash0UndiciInstrumentation from "./instrumentations/undici/UndiciInstrumentation";
import { Dash0AwsSdkV3LibInstrumentation } from './instrumentations/aws-sdk';

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

    const instrumentationsToInstall = [
      new Dash0AmqplibInstrumentation(),
      new Dash0ExpressInstrumentation(),
      new Dash0GrpcInstrumentation(),
      new Dash0NestInstrumentation(),
      new Dash0FastifyInstrumentation(),
      new Dash0HttpInstrumentation(...ignoredHostnames),
      new Dash0IORedisInstrumentation(),
      new Dash0KafkaJsInstrumentation(),
      new Dash0MongoDBInstrumentation(),
      new Dash0Mysql2Instrumentation(),
      new Dash0PgInstrumentation(),
      new Dash0PrismaInstrumentation(),
      new Dash0RedisInstrumentation(),
      new Dash0AwsSdkV3LibInstrumentation(),
      new Dash0UndiciInstrumentation(),
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

    const dashToken = process.env.DASH0_TOKEN || '';

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

    // Build span processors array
    const spanProcessors = [];
    if (process.env.DASH0_DEBUG_SPANDUMP) {
      spanProcessors.push(
        new SimpleSpanProcessor(new FileSpanExporter(process.env.DASH0_DEBUG_SPANDUMP))
      );
    }

    const otlpTraceExporter = new OTLPTraceExporter({
      url: traceEndpoint,
      headers: {
        Authorization: `Bearer ${dashToken.trim()}`,
      },
    });

    spanProcessors.push(
      new BatchSpanProcessor(otlpTraceExporter, {
        // The maximum queue size. After the size is reached spans are dropped.
        maxQueueSize: 1000,
        // The maximum batch size of every export. It must be smaller or equal to maxQueueSize.
        maxExportBatchSize: 100,
      })
    );

    // Create providers with processors
    const tracerProvider = new NodeTracerProvider({
      resource,
      spanLimits: {
        attributeValueLengthLimit: getSpanAttributeMaxLength(),
      },
      spanProcessors,
    });

    tracerProvider.register({
      propagator: new CompositePropagator({
        propagators: [
          new Dash0W3CTraceContextPropagator(),
          new W3CBaggagePropagator(),
          new AWSXRayLambdaPropagator(),
        ],
      }),
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
