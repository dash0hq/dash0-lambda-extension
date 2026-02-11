import { hrTimeToMicroseconds } from '@opentelemetry/core';
import type { ReadableSpan } from '@opentelemetry/sdk-trace-base';
import { FileExporter } from './FileExporter';

/**
 * This is implementation of {@link FileExporter} that prints spans to a file.
 * This class can be used for debug purposes. It is not advised to use this
 * exporter in production.
 */
export class FileSpanExporter extends FileExporter<ReadableSpan> {
  protected exportInfo(span: ReadableSpan): Object {
    return {
      traceId: span.spanContext().traceId,
      parentId: span.parentSpanContext?.spanId,
      name: span.name,
      id: span.spanContext().spanId,
      kind: span.kind,
      timestamp: hrTimeToMicroseconds(span.startTime),
      duration: hrTimeToMicroseconds(span.duration),
      attributes: span.attributes,
      status: span.status,
      events: span.events,
      resource: {
        attributes: span.resource.attributes,
      },
    };
  }
}
