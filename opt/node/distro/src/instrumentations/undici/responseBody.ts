import type { Span } from '@opentelemetry/api';
import { TextDecoder } from 'util';
import { gunzipSync } from 'zlib';
import { getSpanAttributeMaxLength } from '../../utils';

// These are capture limits, not limits on the application's response. In particular,
// a gzip expansion never allocates an unbounded output buffer on the invocation thread.
const ENCODED_LIMIT = 32 * 1024;
const DECODED_LIMIT = 64 * 1024;
const CAPTURE_LIMIT = 16;
let activeCaptures = 0;
const decoder = new TextDecoder('utf-8', { fatal: true });
const nodeMajor = Number(process.versions.node.split('.')[0]);
const undiciVersion = process.versions.undici;
const supported = ([20, 22].includes(nodeMajor) && undiciVersion?.startsWith('6.')) ||
  (nodeMajor === 24 && undiciVersion?.startsWith('7.')) ||
  (nodeMajor === 26 && undiciVersion?.startsWith('8.'));
let cachedSetting: string | undefined;
type MaskingRules = RegExp | 'all' | null;
let cachedRules: MaskingRules | undefined;

const DEFAULT_RULES = [
  '.*pass.*', '.*key.*', '.*secret.*', '.*credential.*', '.*passphrase.*',
  '.*token.*', '.*signature.*', '.*authorization.*',
];

// Do not run arbitrary customer regexes on response-controlled keys. Even a small
// body can trigger exponential backtracking. Recognize a deliberately small subset
// as literal alternatives with optional anchors, never customer-supplied regex syntax.
// The generated matcher has no quantifiers, captures or backreferences.
function maskingRules(): MaskingRules | undefined {
  if (process.env.LUMIGO_WHITELIST_KEYS_REGEXES) return;
  const setting = process.env.LUMIGO_SECRET_MASKING_REGEX_HTTP_RESPONSE_BODIES ||
    process.env.LUMIGO_SECRET_MASKING_REGEX || process.env.LUMIGO_BLACKLIST_REGEX;
  if (setting === 'all') return 'all';
  if (setting === cachedSetting && cachedRules !== undefined) return cachedRules;
  if (setting && setting.length > 4096) return;
  const patterns = setting ? JSON.parse(setting) : DEFAULT_RULES;
  if (!Array.isArray(patterns) || patterns.length > 32) return;
  const alternatives: string[] = [];
  for (const pattern of patterns) {
    if (typeof pattern !== 'string' || pattern.length > 128) return;
    const match = /^(\^)?(\.\*)?([a-zA-Z0-9_ -]+)(\.\*)?(\$)?$/.exec(pattern);
    if (!match) return;
    const start = !!match[1] && !match[2];
    const end = !!match[5] && !match[4];
    alternatives.push(`${start ? '^' : ''}${match[3]}${end ? '$' : ''}`);
  }
  cachedSetting = setting;
  return cachedRules = alternatives.length ? new RegExp(alternatives.join('|'), 'i') : null;
}

function maskBody(bytes: Buffer, rules: MaskingRules): string | undefined {
  // Invalid UTF-8 and invalid JSON are omitted, not exported as an unmasked string.
  const payload = JSON.parse(decoder.decode(bytes));
  if (rules === 'all') return '"****"';
  const matcher = rules;
  let keys = 0;
  // Only recurse into objects, with a hard depth bound. Do not allocate a pending
  // entry or a matching closure for every primitive value in a wide JSON array.
  function visit(value: any, depth: number): boolean {
    if (value === null || typeof value !== 'object') return true;
    if (depth > 32) return false;
    for (const key of Object.keys(value)) {
      if (++keys > 2048 || key.length > 128) return false;
      if (matcher?.test(key)) {
        // JSON.parse creates own writable data properties, including "__proto__".
        value[key] = '****';
      } else if (!visit(value[key], depth + 1)) {
        return false;
      }
    }
    return true;
  }
  if (!visit(payload, 0)) return;
  const result = JSON.stringify(payload);
  // No truncation: masking and serialization must finish before checking the export cap.
  return result.length <= Math.min(getSpanAttributeMaxLength(), DECODED_LIMIT) ? result : undefined;
}

// Undici 8 renamed the Request callbacks. Their arguments (chunk, trailers, error) are unchanged.
const callbackNames = undiciVersion?.startsWith('8.')
  ? ['onResponseData', 'onResponseEnd', 'onResponseError'] as const
  : ['onData', 'onComplete', 'onError'] as const;
const [DATA, COMPLETE, ERROR] = callbackNames;
type RequestCallbacks = Record<(typeof callbackNames)[number], (...args: any[]) => any>;

type CaptureState = {
  span?: Span;
  buffer?: Buffer;
  size: number;
  released: boolean;
  encodedLimit: number;
  decodedLimit: number;
  length?: number;
  gzip: boolean;
  rules: MaskingRules;
  callbacks: PropertyDescriptorMap;
};
const states = new WeakMap<RequestCallbacks, CaptureState>();
type Callback = (...args: any[]) => any;
const callbackCache = new WeakMap<object, { originals: Callback[]; callbacks: PropertyDescriptorMap }>();

function releaseCapture(target: RequestCallbacks, state: CaptureState): void {
  if (state.released) return;
  state.released = true;
  activeCaptures--;
  state.buffer = undefined;
  state.span = undefined;
  try { states.delete(target); } catch { /* Cleanup must not escape into Undici. */ }
  for (const name of callbackNames) {
    try {
      if (target[name] === state.callbacks[name].value) delete target[name];
    } catch { /* A caller may have frozen the request or replaced a callback. */ }
  }
}

// Share wrappers and descriptors per Request prototype, not per response. Originals
// live in this prototype's closure so even a borrowed wrapper preserves its receiver
// and arguments. An unsupported receiver simply receives the original callback.
function callbackDescriptors(target: RequestCallbacks): PropertyDescriptorMap | undefined {
  const prototype = Object.getPrototypeOf(target);
  const onData = target[DATA], onComplete = target[COMPLETE], onError = target[ERROR];
  const cached = callbackCache.get(prototype);
  if (cached) {
    if (cached.originals[0] !== onData || cached.originals[1] !== onComplete ||
      cached.originals[2] !== onError) return;
    return cached.callbacks;
  }
  const callbacks = {
    [DATA]: { configurable: true, writable: true, value: function captureOnData(this: RequestCallbacks) {
      let state: CaptureState | undefined;
      try {
        state = states.get(this);
        if (state && !state.released) {
          const chunk = arguments[0];
          if (!Buffer.isBuffer(chunk) || state.size + chunk.length > state.encodedLimit ||
            (state.length !== undefined && state.size + chunk.length > state.length)) {
            releaseCapture(this, state);
          } else if (state.gzip && state.size === 0 && chunk.length === state.length &&
            chunk.length >= 18 && chunk.readUInt32LE(chunk.length - 4) > state.decodedLimit) {
            // Negative hint only: never use ISIZE to accept a gzip stream. Accepted
            // streams still require a complete gunzip with enforced maxOutputLength.
            releaseCapture(this, state);
          } else {
            state.buffer ??= Buffer.allocUnsafe(state.length ?? state.encodedLimit);
            if (chunk.copy(state.buffer, state.size) !== chunk.length) releaseCapture(this, state);
            else state.size += chunk.length;
          }
        }
      } catch { if (state) releaseCapture(this, state); }
      try { return onData.apply(this, arguments); }
      catch (error) { if (state) releaseCapture(this, state); throw error; }
    } },
    [COMPLETE]: { configurable: true, writable: true, value: function captureOnComplete(this: RequestCallbacks) {
      let state: CaptureState | undefined;
      try {
        state = states.get(this);
        if (state && !state.released && state.buffer &&
          (state.length === undefined || state.length === state.size)) {
          // Only the initialized prefix is readable, never allocUnsafe's unused tail.
          const encoded = state.buffer.subarray(0, state.size);
          const decoded = state.gzip
            ? state.size >= 18 && encoded.readUInt32LE(state.size - 4) <= state.decodedLimit
              ? gunzipSync(encoded, { maxOutputLength: state.decodedLimit,
                chunkSize: Math.max(64, Math.min(state.decodedLimit, 16 * 1024)) }) : undefined
            : encoded;
          const body = decoded ? maskBody(decoded, state.rules) : undefined;
          if (body !== undefined) state.span?.setAttribute('http.response.body', body);
        }
      } catch { /* Omit capture on decoding, masking or span errors. */ }
      finally { if (state) releaseCapture(this, state); }
      // The original publishes trailers and ends the OTel span synchronously.
      return onComplete.apply(this, arguments);
    } },
    [ERROR]: { configurable: true, writable: true, value: function captureOnError(this: RequestCallbacks) {
      try {
        const state = states.get(this);
        if (state) releaseCapture(this, state);
      } catch { /* Preserve the original error path even if capture cleanup fails. */ }
      return onError.apply(this, arguments);
    } },
  };
  callbackCache.set(prototype, { originals: [onData, onComplete, onError], callbacks });
  return callbacks;
}

/** Installs only on the tested Undici Request callback contract, never on a Fetch body. */
export function captureResponseBody(
  request: unknown,
  span: Span,
  headers: Record<string, string>
): void {
  try {
    if (!supported) return;
    if (!span.isRecording() || activeCaptures >= CAPTURE_LIMIT) return;
    const mediaType = headers['content-type']?.split(';')[0].trim().toLowerCase();
    if (mediaType !== 'application/json' && mediaType !== 'application/graphql-response+json') return;
    const encoding = (headers['content-encoding'] || 'identity').trim().toLowerCase();
    if (encoding !== 'identity' && encoding !== 'gzip') return;
    // Do not decode/parse bodies too large to export. This deliberately sacrifices
    // bodies that might become small after masking to bound invocation-thread work.
    const decodedLimit = Math.min(getSpanAttributeMaxLength(), DECODED_LIMIT);
    if (decodedLimit < 1) return;
    const encodedLimit = encoding === 'identity' ? Math.min(ENCODED_LIMIT, decodedLimit) : ENCODED_LIMIT;
    const length = headers['content-length'];
    if (length && (!/^\d+$/.test(length) || Number(length) > encodedLimit)) return;
    const rules = maskingRules();
    if (rules === undefined) return;

    const target = request as RequestCallbacks;
    // Undici's Request inherits these methods. Own methods indicate another wrapper
    // or an unknown implementation. Do not overwrite it, or partially patch a request.
    if (!target || !Object.isExtensible(target)) return;
    for (const name of callbackNames) {
      if (Object.prototype.hasOwnProperty.call(target, name) || typeof target[name] !== 'function') return;
    }
    const callbacks = callbackDescriptors(target);
    if (!callbacks) return;
    const state: CaptureState = {
      span, size: 0, released: false, encodedLimit, decodedLimit,
      length: length === undefined ? undefined : Number(length), gzip: encoding === 'gzip', rules, callbacks,
    };
    activeCaptures++;
    try {
      states.set(target, state);
      Object.defineProperties(target, callbacks);
    } catch { releaseCapture(target, state); }
  } catch { /* Unknown internals or configuration: leave HTTP operational. */ }
}
