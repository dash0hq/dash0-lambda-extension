import * as crypto from 'crypto';
import * as http from 'http';
import * as https from 'https';

import { logger } from './logging';
import { sortify } from './tools/jsonSortify';

export const DEFAULT_ATTRIBUTE_VALUE_LENGTH_LIMIT = 2048;

export function safeExecute<T>(
  callback: Function,
  message = 'Error in Lumigo tracer',
  defaultReturn: T = undefined
): Function {
  return function (...args) {
    try {
      return callback.apply(this, args);
    } catch (err) {
      logger.debug(message, err);
      return defaultReturn;
    }
  };
}

export const isEncodingType = (encodingType): boolean =>
  !!(
    encodingType &&
    typeof encodingType === 'string' &&
    ['ascii', 'utf8', 'utf16le', 'ucs2', 'base64', 'binary', 'hex'].includes(encodingType)
  );

export const runOneTimeWrapper = (func: Function, context: any = undefined): Function => {
  let done = false;
  return (...args) => {
    if (!done) {
      const result = func.apply(context || this, args);
      done = true;
      return result;
    }
  };
};

export const safeGet = (obj, arr, dflt = null) => {
  let current = obj;
  for (const i in arr) {
    if (!current) {
      return dflt;
    }
    current = current[arr[i]];
  }
  return current || dflt;
};

export const safeParse = (parseable) => {
  try {
    return JSON.parse(parseable);
  } catch (e) {
    return parseable;
  }
};

export const parseQueryParams = (queryParams) => {
  return safeExecute(() => {
    if (typeof queryParams !== 'string') return {};
    try {
      return JSON.parse(queryParams);
    } catch (e) {
      // ignore because sqs request body could also be not a JSON
    }
    const obj = {};
    queryParams.replace(
      /([^=&]+)=([^&]*)/g,
      // @ts-ignore
      safeExecute((m, key, value) => {
        obj[decodeURIComponent(key)] = decodeURIComponent(value);
      }, 'Failed to parse a specific key in parseQueryParams')
    );
    return obj;
  }, 'Failed to parse query params')();
};

export const md5Hash = (item: {}): string | undefined => {
  try {
    const md5sum = crypto.createHash('md5');
    md5sum.update(sortify(item));
    return md5sum.digest('hex');
  } catch (err) {
    logger.debug('Failed to hash item', err);
    return undefined;
  }
};

// @ts-ignore
export const removeDuplicates = (arr) => Array.from(new Set(arr));

export const getSpanAttributeMaxLength = () => {
  return (
    parseInt(process.env.OTEL_SPAN_ATTRIBUTE_VALUE_LENGTH_LIMIT) ||
    parseInt(process.env.OTEL_ATTRIBUTE_VALUE_LENGTH_LIMIT) ||
    DEFAULT_ATTRIBUTE_VALUE_LENGTH_LIMIT
  );
};

// For now, we use the same set of limits for logs and spans, therefore this is just an alias
export const getLogAttributeMaxLength = getSpanAttributeMaxLength;

export const getResourceAttributeMaxLength = () => {
  return (
    parseInt(process.env.OTEL_ATTRIBUTE_VALUE_LENGTH_LIMIT) || DEFAULT_ATTRIBUTE_VALUE_LENGTH_LIMIT
  );
};
