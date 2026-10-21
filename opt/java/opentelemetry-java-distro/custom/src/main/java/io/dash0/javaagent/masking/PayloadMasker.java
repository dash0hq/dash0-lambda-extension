/*
 * Copyright 2023 Dash0 LTD
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *   http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 *
 * SPDX-License-Identifier: Apache-2.0
 */
package io.dash0.javaagent.masking;

import static io.dash0.javaagent.masking.MaskingRules.MASKED_VALUE;

import com.fasterxml.jackson.core.JsonFactory;
import com.fasterxml.jackson.core.JsonGenerator;
import com.fasterxml.jackson.core.JsonParser;
import com.fasterxml.jackson.core.JsonToken;
import java.io.IOException;
import java.io.StringWriter;
import java.net.URLDecoder;
import java.nio.charset.StandardCharsets;

/** Masks the string payloads the instrumentations capture. */
final class PayloadMasker {
  private static final JsonFactory JSON_FACTORY = new JsonFactory();

  private PayloadMasker() {}

  /** Masks a payload that is never parsed, such as a Kafka message: only {@code all} applies. */
  static String maskString(String payload, MaskingRules rules) {
    return rules.masksAll() ? MASKED_VALUE : payload;
  }

  /**
   * Masks a body. A body that is a JSON object or array is masked by key, at any depth, and written
   * back as compact JSON. Any other body is returned as is, unless the rules are {@code all}.
   *
   * <p>Bodies longer than the span attribute length limit arrive truncated, so they are no longer
   * valid JSON. The part before the cut is still masked; the value that was cut off is dropped and
   * the result is closed so that it is valid JSON again.
   */
  static String maskJson(String body, MaskingRules rules) {
    if (rules.masksAll()) {
      return MASKED_VALUE;
    }
    StringWriter writer = new StringWriter(body.length());
    boolean changed = false;
    try (JsonParser parser = JSON_FACTORY.createParser(body);
        JsonGenerator generator = JSON_FACTORY.createGenerator(writer)) {
      JsonToken first;
      try {
        first = parser.nextToken();
      } catch (IOException e) {
        return body;
      }
      if (first != JsonToken.START_OBJECT && first != JsonToken.START_ARRAY) {
        return body;
      }
      generator.copyCurrentEvent(parser);
      String pendingName = null;
      try {
        JsonToken token;
        while (parser.getParsingContext().getParent() != null
            && (token = parser.nextToken()) != null) {
          if (token == JsonToken.FIELD_NAME) {
            String name = parser.currentName();
            if (rules.matches(name)) {
              generator.writeStringField(name, MASKED_VALUE);
              changed = true;
              parser.nextToken();
              parser.skipChildren();
            } else {
              pendingName = name;
            }
            continue;
          }
          if (token.isScalarValue()) {
            // Reads the whole value, so that a value cut off by truncation fails here, before its
            // key is written.
            parser.getText();
          }
          if (pendingName != null) {
            generator.writeFieldName(pendingName);
            pendingName = null;
          }
          generator.copyCurrentEventExact(parser);
        }
      } catch (IOException truncated) {
        // Keep what was written so far. If a key was masked, closing the generator closes the
        // open objects and arrays; otherwise the body is returned as it was.
      }
    } catch (IOException e) {
      return body;
    }
    return changed ? writer.toString() : body;
  }

  /**
   * Masks the values of the query parameters whose names match, and keeps the query structure:
   * {@code a=1&password=x} becomes {@code a=1&password=****}.
   */
  static String maskQuery(String query, MaskingRules rules) {
    if (rules.masksAll()) {
      return MASKED_VALUE;
    }
    String[] params = query.split("&", -1);
    boolean changed = false;
    for (int i = 0; i < params.length; i++) {
      String param = params[i];
      int eq = param.indexOf('=');
      String name = eq >= 0 ? param.substring(0, eq) : param;
      if (!name.isEmpty() && rules.matches(decode(name))) {
        params[i] = name + "=" + MASKED_VALUE;
        changed = true;
      }
    }
    return changed ? String.join("&", params) : query;
  }

  /** Masks the query string of a URL with {@link #maskQuery}. */
  static String maskUrl(String url, MaskingRules rules) {
    int queryStart = url.indexOf('?');
    if (queryStart < 0) {
      return url;
    }
    int fragmentStart = url.indexOf('#', queryStart);
    int queryEnd = fragmentStart >= 0 ? fragmentStart : url.length();
    String query = url.substring(queryStart + 1, queryEnd);
    if (query.isEmpty()) {
      return url;
    }
    String masked = maskQuery(query, rules);
    if (masked.equals(query)) {
      return url;
    }
    return url.substring(0, queryStart + 1) + masked + url.substring(queryEnd);
  }

  private static String decode(String value) {
    try {
      return URLDecoder.decode(value, StandardCharsets.UTF_8.name());
    } catch (Exception e) {
      return value;
    }
  }
}
