let capturedConfig: { requestHook: Function; responseHook: Function } | undefined;

jest.mock('@opentelemetry/instrumentation-undici', () => ({
  UndiciInstrumentation: jest.fn().mockImplementation((config: any) => {
    capturedConfig = config;
    return {};
  }),
}));

jest.mock('@lumigo/node-core', () => ({
  CommonUtils: {
    payloadStringify: jest.fn((obj: any) => JSON.stringify(obj)),
  },
  ScrubContext: {
    HTTP_REQUEST_HEADERS: 'requestHeaders',
    HTTP_RESPONSE_HEADERS: 'responseHeaders',
    HTTP_REQUEST_BODY: 'requestBody',
  },
}));

jest.mock('../../tools/payloads', () => ({
  scrubHttpPayload: jest.fn((body: string) => body),
}));

function createMockSpan() {
  const attributes: Record<string, any> = {};
  return {
    setAttribute: jest.fn((key: string, value: any) => {
      attributes[key] = value;
    }),
    attributes,
  };
}

describe('Dash0UndiciInstrumentation', () => {
  const origFetch = globalThis.fetch;

  beforeEach(() => {
    jest.resetModules();
    capturedConfig = undefined;
  });

  afterEach(() => {
    globalThis.fetch = origFetch;
    jest.clearAllMocks();
  });

  function loadModule() {
    return require('./UndiciInstrumentation').default;
  }

  /** Load a fresh module, call getInstrumentation, and return the captured hooks. */
  function getHooks() {
    globalThis.fetch = jest.fn().mockResolvedValue({}) as any;
    const Cls = loadModule();
    new Cls().getInstrumentation();
    return {
      requestHook: capturedConfig!.requestHook,
      responseHook: capturedConfig!.responseHook,
    };
  }

  describe('class API', () => {
    test('getInstrumentedModule returns "fetch"', () => {
      const Cls = loadModule();
      expect(new Cls().getInstrumentedModule()).toBe('fetch');
    });

    test('isApplicable returns true', () => {
      const Cls = loadModule();
      expect(new Cls().isApplicable()).toBe(true);
    });
  });

  describe('requestHook', () => {
    describe('header parsing', () => {
      test('parses v5 string format headers', () => {
        const { requestHook } = getHooks();
        const span = createMockSpan();

        requestHook(span, {
          headers: 'Content-Type: application/json\r\nX-Custom: value\r\n',
        });

        const headers = JSON.parse(span.attributes['http.request.headers']);
        expect(headers['content-type']).toBe('application/json');
        expect(headers['x-custom']).toBe('value');
      });

      test('parses v6 array format headers', () => {
        const { requestHook } = getHooks();
        const span = createMockSpan();

        requestHook(span, {
          headers: ['Content-Type', 'application/json', 'X-Custom', 'value'],
        });

        const headers = JSON.parse(span.attributes['http.request.headers']);
        expect(headers['content-type']).toBe('application/json');
        expect(headers['x-custom']).toBe('value');
      });

      test('lowercases header names in v5 format', () => {
        const { requestHook } = getHooks();
        const span = createMockSpan();

        requestHook(span, { headers: 'X-UPPER-CASE: value\r\n' });

        const headers = JSON.parse(span.attributes['http.request.headers']);
        expect(headers['x-upper-case']).toBe('value');
      });

      test('lowercases header names in v6 format', () => {
        const { requestHook } = getHooks();
        const span = createMockSpan();

        requestHook(span, { headers: ['X-UPPER-CASE', 'value'] });

        const headers = JSON.parse(span.attributes['http.request.headers']);
        expect(headers['x-upper-case']).toBe('value');
      });

      test('handles header value containing colons', () => {
        const { requestHook } = getHooks();
        const span = createMockSpan();

        requestHook(span, { headers: 'Authorization: Bearer abc:def:ghi\r\n' });

        const headers = JSON.parse(span.attributes['http.request.headers']);
        expect(headers['authorization']).toBe('Bearer abc:def:ghi');
      });

      test('handles empty string headers', () => {
        const { requestHook } = getHooks();
        const span = createMockSpan();

        expect(() => requestHook(span, { headers: '' })).not.toThrow();
        expect(span.setAttribute).toHaveBeenCalledWith(
          'http.request.headers',
          expect.any(String)
        );
      });

      test('handles empty array headers', () => {
        const { requestHook } = getHooks();
        const span = createMockSpan();

        expect(() => requestHook(span, { headers: [] })).not.toThrow();
        expect(span.setAttribute).toHaveBeenCalledWith(
          'http.request.headers',
          expect.any(String)
        );
      });
    });

    describe('body capture', () => {
      test('captures string body from request', () => {
        const { requestHook } = getHooks();
        const span = createMockSpan();

        requestHook(span, {
          headers: '',
          body: '{"key":"value"}',
          contentType: 'application/json',
        });

        expect(span.setAttribute).toHaveBeenCalledWith(
          'http.request.body',
          '{"key":"value"}'
        );
      });

      test('captures Buffer body from request', () => {
        const { requestHook } = getHooks();
        const span = createMockSpan();

        requestHook(span, {
          headers: '',
          body: Buffer.from('buffer-body'),
          contentType: 'text/plain',
        });

        expect(span.setAttribute).toHaveBeenCalledWith(
          'http.request.body',
          'buffer-body'
        );
      });

      test('does not set body when body is undefined', () => {
        const { requestHook } = getHooks();
        const span = createMockSpan();

        requestHook(span, { headers: '' });

        expect(span.setAttribute).not.toHaveBeenCalledWith(
          'http.request.body',
          expect.anything()
        );
      });

      test('does not set body for non-string non-Buffer body (e.g. AsyncGenerator)', () => {
        const { requestHook } = getHooks();
        const span = createMockSpan();

        requestHook(span, {
          headers: '',
          body: { [Symbol.asyncIterator]: () => {} },
        });

        expect(span.setAttribute).not.toHaveBeenCalledWith(
          'http.request.body',
          expect.anything()
        );
      });

      test('uses content-type from headers when request.contentType is missing', () => {
        const { requestHook } = getHooks();
        const span = createMockSpan();
        const { scrubHttpPayload } = require('../../tools/payloads');

        requestHook(span, {
          headers: 'Content-Type: text/xml\r\n',
          body: '<xml/>',
        });

        expect(scrubHttpPayload).toHaveBeenCalledWith(
          '<xml/>',
          'text/xml',
          expect.anything()
        );
      });

      test('prefers request.contentType over header content-type', () => {
        const { requestHook } = getHooks();
        const span = createMockSpan();
        const { scrubHttpPayload } = require('../../tools/payloads');

        requestHook(span, {
          headers: 'Content-Type: text/plain\r\n',
          body: '{}',
          contentType: 'application/json',
        });

        expect(scrubHttpPayload).toHaveBeenCalledWith(
          '{}',
          'application/json',
          expect.anything()
        );
      });
    });

    describe('error resilience', () => {
      test('does not throw when span.setAttribute throws', () => {
        const { requestHook } = getHooks();
        const throwingSpan = {
          setAttribute: jest.fn(() => {
            throw new Error('span error');
          }),
        };

        expect(() =>
          requestHook(throwingSpan, {
            headers: '',
            body: 'data',
            contentType: 'text/plain',
          })
        ).not.toThrow();
      });

      test('does not throw when request is null', () => {
        const { requestHook } = getHooks();
        const span = createMockSpan();

        expect(() => requestHook(span, null)).not.toThrow();
      });

      test('does not throw when request is undefined', () => {
        const { requestHook } = getHooks();
        const span = createMockSpan();

        expect(() => requestHook(span, undefined)).not.toThrow();
      });

      test('does not throw with null headers', () => {
        const { requestHook } = getHooks();
        const span = createMockSpan();

        expect(() => requestHook(span, { headers: null })).not.toThrow();
      });

      test('does not throw with numeric headers', () => {
        const { requestHook } = getHooks();
        const span = createMockSpan();

        expect(() => requestHook(span, { headers: 42 })).not.toThrow();
      });
    });
  });

  describe('responseHook', () => {
    test('parses Buffer array headers', () => {
      const { responseHook } = getHooks();
      const span = createMockSpan();

      responseHook(span, {
        request: {},
        response: {
          headers: [
            Buffer.from('Content-Type'),
            Buffer.from('application/json'),
            Buffer.from('X-Request-Id'),
            Buffer.from('abc123'),
          ],
        },
      });

      const headers = JSON.parse(span.attributes['http.response.headers']);
      expect(headers['content-type']).toBe('application/json');
      expect(headers['x-request-id']).toBe('abc123');
    });

    test('handles empty headers array', () => {
      const { responseHook } = getHooks();
      const span = createMockSpan();

      expect(() =>
        responseHook(span, { request: {}, response: { headers: [] } })
      ).not.toThrow();
      expect(span.setAttribute).toHaveBeenCalledWith(
        'http.response.headers',
        expect.any(String)
      );
    });

    describe('error resilience', () => {
      test('does not throw with non-array headers', () => {
        const { responseHook } = getHooks();
        const span = createMockSpan();

        expect(() =>
          responseHook(span, {
            request: {},
            response: { headers: 'not-an-array' },
          })
        ).not.toThrow();
      });

      test('does not throw with null response', () => {
        const { responseHook } = getHooks();
        const span = createMockSpan();

        expect(() =>
          responseHook(span, { request: {}, response: null })
        ).not.toThrow();
      });

      test('does not throw when span.setAttribute throws', () => {
        const { responseHook } = getHooks();
        const throwingSpan = {
          setAttribute: jest.fn(() => {
            throw new Error('span error');
          }),
        };

        expect(() =>
          responseHook(throwingSpan, {
            request: {},
            response: {
              headers: [Buffer.from('k'), Buffer.from('v')],
            },
          })
        ).not.toThrow();
      });
    });
  });

  describe('fetch wrapper', () => {
    test('wraps globalThis.fetch when it exists', () => {
      const mockFetch = jest.fn().mockResolvedValue({});
      globalThis.fetch = mockFetch as any;

      const Cls = loadModule();
      new Cls().getInstrumentation();

      expect(globalThis.fetch).not.toBe(mockFetch);
    });

    test('does not wrap when globalThis.fetch is not a function', () => {
      (globalThis as any).fetch = 'not-a-function';

      const Cls = loadModule();
      new Cls().getInstrumentation();

      expect(globalThis.fetch).toBe('not-a-function' as any);
    });

    test('does not wrap when globalThis.fetch is undefined', () => {
      delete (globalThis as any).fetch;

      const Cls = loadModule();
      new Cls().getInstrumentation();

      expect(globalThis.fetch).toBeUndefined();
    });

    test('calls through to original fetch and returns its result', async () => {
      const mockResponse = { status: 200 };
      const mockFetch = jest.fn().mockResolvedValue(mockResponse);
      globalThis.fetch = mockFetch as any;

      const Cls = loadModule();
      new Cls().getInstrumentation();

      const result = await globalThis.fetch('https://example.com');
      expect(mockFetch).toHaveBeenCalled();
      expect(result).toBe(mockResponse);
    });

    test('preserves all arguments to original fetch', async () => {
      const mockFetch = jest.fn().mockResolvedValue({});
      globalThis.fetch = mockFetch as any;

      const Cls = loadModule();
      new Cls().getInstrumentation();

      const init = {
        method: 'POST',
        body: 'test',
        headers: { 'X-Custom': 'value' },
      };
      await globalThis.fetch('https://example.com', init);

      expect(mockFetch).toHaveBeenCalledWith('https://example.com', init);
    });

    test('propagates async errors from original fetch', async () => {
      const mockFetch = jest.fn().mockRejectedValue(new Error('network error'));
      globalThis.fetch = mockFetch as any;

      const Cls = loadModule();
      new Cls().getInstrumentation();

      await expect(globalThis.fetch('https://example.com')).rejects.toThrow(
        'network error'
      );
    });

    test('propagates sync errors from original fetch', () => {
      const mockFetch = jest.fn(() => {
        throw new Error('sync error');
      });
      globalThis.fetch = mockFetch as any;

      const Cls = loadModule();
      new Cls().getInstrumentation();

      expect(() => globalThis.fetch('https://example.com')).toThrow(
        'sync error'
      );
    });

    test('does not wrap fetch twice on multiple getInstrumentation calls', () => {
      const mockFetch = jest.fn().mockResolvedValue({});
      globalThis.fetch = mockFetch as any;

      const Cls = loadModule();
      const inst = new Cls();
      inst.getInstrumentation();
      const wrappedOnce = globalThis.fetch;

      inst.getInstrumentation();
      expect(globalThis.fetch).toBe(wrappedOnce);
    });

    test('wraps fetch on retry if it was unavailable on first call', () => {
      delete (globalThis as any).fetch;

      const Cls = loadModule();
      const inst = new Cls();
      inst.getInstrumentation();
      expect(globalThis.fetch).toBeUndefined();

      // Now install fetch and retry
      const mockFetch = jest.fn().mockResolvedValue({});
      globalThis.fetch = mockFetch as any;

      inst.getInstrumentation();
      expect(globalThis.fetch).not.toBe(mockFetch);
    });

    describe('body extraction safety', () => {
      function setupWrappedFetch() {
        const mockFetch = jest.fn().mockResolvedValue({});
        globalThis.fetch = mockFetch as any;

        const Cls = loadModule();
        new Cls().getInstrumentation();

        return mockFetch;
      }

      test('handles string body without crashing', async () => {
        const mockFetch = setupWrappedFetch();
        await globalThis.fetch('https://example.com', { body: 'hello' });
        expect(mockFetch).toHaveBeenCalled();
      });

      test('handles Buffer body without crashing', async () => {
        const mockFetch = setupWrappedFetch();
        await globalThis.fetch('https://example.com', {
          body: Buffer.from('data') as any,
        });
        expect(mockFetch).toHaveBeenCalled();
      });

      test('handles ArrayBuffer body without crashing', async () => {
        const mockFetch = setupWrappedFetch();
        await globalThis.fetch('https://example.com', {
          body: new ArrayBuffer(4) as any,
        });
        expect(mockFetch).toHaveBeenCalled();
      });

      test('handles Uint8Array body without crashing', async () => {
        const mockFetch = setupWrappedFetch();
        await globalThis.fetch('https://example.com', {
          body: new Uint8Array([1, 2, 3]) as any,
        });
        expect(mockFetch).toHaveBeenCalled();
      });

      test('handles URLSearchParams body without crashing', async () => {
        const mockFetch = setupWrappedFetch();
        await globalThis.fetch('https://example.com', {
          body: new URLSearchParams({ key: 'value' }),
        });
        expect(mockFetch).toHaveBeenCalled();
      });

      test('handles null body without crashing', async () => {
        const mockFetch = setupWrappedFetch();
        await globalThis.fetch('https://example.com', { body: null });
        expect(mockFetch).toHaveBeenCalled();
      });

      test('handles undefined init without crashing', async () => {
        const mockFetch = setupWrappedFetch();
        await globalThis.fetch('https://example.com');
        expect(mockFetch).toHaveBeenCalled();
      });

      test('handles ReadableStream body without crashing', async () => {
        const mockFetch = setupWrappedFetch();
        await globalThis.fetch('https://example.com', {
          body: new ReadableStream(),
        });
        expect(mockFetch).toHaveBeenCalled();
      });

      test('does not crash fetch when extractBody encounters a throwing Proxy', async () => {
        const mockFetch = setupWrappedFetch();
        const evilBody = new Proxy(new Uint8Array(), {
          get() {
            throw new Error('proxy trap');
          },
          getPrototypeOf() {
            throw new Error('proxy trap');
          },
        });

        await globalThis.fetch('https://example.com', {
          body: evilBody as any,
        });
        expect(mockFetch).toHaveBeenCalled();
      });

      test('still calls original fetch even when body extraction fails', async () => {
        const mockFetch = setupWrappedFetch();
        const badBody = {
          get toString() {
            throw new Error('toString trap');
          },
        };

        await globalThis.fetch('https://example.com', {
          body: badBody as any,
        });
        expect(mockFetch).toHaveBeenCalled();
      });
    });
  });
});
