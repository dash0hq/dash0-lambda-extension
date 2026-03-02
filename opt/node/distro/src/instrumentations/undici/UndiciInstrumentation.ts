import { CommonUtils, ScrubContext } from '@lumigo/node-core';
import type { Span } from '@opentelemetry/api';
import { UndiciInstrumentation } from '@opentelemetry/instrumentation-undici';
import type { UndiciRequest, UndiciResponse } from '@opentelemetry/instrumentation-undici';
import { scrubHttpPayload } from '../../tools/payloads';
import { getSpanAttributeMaxLength, safeExecute } from '../../utils';
import { TracingInstrumentor } from '../instrumentor';

function parseRequestHeaders(headers: UndiciRequest['headers']): Record<string, string> {
  const result: Record<string, string> = {};
  if (typeof headers === 'string') {
    // v5 format: serialized "key: value\r\n" pairs
    for (const line of headers.split('\r\n')) {
      const colonIndex = line.indexOf(':');
      if (colonIndex > 0) {
        result[line.substring(0, colonIndex).trim().toLowerCase()] =
          line.substring(colonIndex + 1).trim();
      }
    }
  } else if (Array.isArray(headers)) {
    // v6 format: [key1, value1, key2, value2, ...]
    for (let i = 0; i < headers.length - 1; i += 2) {
      result[String(headers[i]).toLowerCase()] = String(headers[i + 1]);
    }
  }
  return result;
}

function parseResponseHeaders(headers: Buffer[]): Record<string, string> {
  const result: Record<string, string> = {};
  if (!Array.isArray(headers)) return result;
  for (let i = 0; i < headers.length - 1; i += 2) {
    result[headers[i].toString().toLowerCase()] = headers[i + 1].toString();
  }
  return result;
}

// Capture the original fetch body before undici wraps it in an AsyncGenerator.
// The undici diagnostics channel fires synchronously during fetch(), so the
// requestHook reads this while it's still set. The finally block in the wrapper
// clears it immediately after the synchronous part of fetch() completes.
let capturedFetchBody: string | undefined;

function extractBody(body: unknown): string | undefined {
  if (body == null) return undefined;
  if (typeof body === 'string') return body;
  if (Buffer.isBuffer(body)) return body.toString();
  if (body instanceof ArrayBuffer || body instanceof Uint8Array) {
    return Buffer.from(body as ArrayBuffer).toString();
  }
  if (body instanceof URLSearchParams) return body.toString();
  return undefined;
}

let fetchWrapped = false;

function wrapFetch() {
  if (fetchWrapped || typeof globalThis.fetch !== 'function') return;

  const origFetch = globalThis.fetch;
  try {
    globalThis.fetch = function (
      input: RequestInfo | URL,
      init?: RequestInit
    ): Promise<Response> {
      safeExecute(() => {
        capturedFetchBody = extractBody(init?.body);
      })();
      try {
        return origFetch.apply(globalThis, arguments as unknown as Parameters<typeof fetch>);
      } finally {
        capturedFetchBody = undefined;
      }
    };
    fetchWrapped = true;
  } catch {
    globalThis.fetch = origFetch;
  }
}

export default class Dash0UndiciInstrumentation extends TracingInstrumentor<UndiciInstrumentation> {
  override isApplicable(): boolean {
    return true;
  }

  getInstrumentedModule(): string {
    return 'fetch';
  }

  getInstrumentation(): UndiciInstrumentation {
    wrapFetch();

    return new UndiciInstrumentation({
      requestHook(span: Span, request: UndiciRequest) {
        safeExecute(() => {
          const headers = parseRequestHeaders(request.headers);
          span.setAttribute(
            'http.request.headers',
            CommonUtils.payloadStringify(
              headers,
              ScrubContext.HTTP_REQUEST_HEADERS,
              getSpanAttributeMaxLength()
            )
          );

          // Use the body captured from our fetch wrapper (before undici wraps it in an AsyncGenerator).
          // Fall back to request.body for cases where it's already materialized (e.g. older undici versions).
          const body = capturedFetchBody ?? (
            typeof request.body === 'string' || Buffer.isBuffer(request.body)
              ? request.body.toString()
              : undefined
          );
          if (body != null) {
            span.setAttribute(
              'http.request.body',
              scrubHttpPayload(
                body,
                request.contentType || headers['content-type'],
                ScrubContext.HTTP_REQUEST_BODY
              )
            );
          }
        }, 'Error in undici request hook')();
      },

      responseHook(
        span: Span,
        { response }: { request: UndiciRequest; response: UndiciResponse }
      ) {
        safeExecute(() => {
          const headers = parseResponseHeaders(response.headers);
          span.setAttribute(
            'http.response.headers',
            CommonUtils.payloadStringify(
              headers,
              ScrubContext.HTTP_RESPONSE_HEADERS,
              getSpanAttributeMaxLength()
            )
          );
        }, 'Error in undici response hook')();
      },
    });
  }
}
