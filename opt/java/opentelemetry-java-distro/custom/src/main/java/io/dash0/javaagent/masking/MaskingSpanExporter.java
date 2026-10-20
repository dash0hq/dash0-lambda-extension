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

import io.opentelemetry.api.common.AttributeKey;
import io.opentelemetry.api.common.AttributeType;
import io.opentelemetry.api.common.Attributes;
import io.opentelemetry.api.common.AttributesBuilder;
import io.opentelemetry.api.trace.SpanKind;
import io.opentelemetry.sdk.common.CompletableResultCode;
import io.opentelemetry.sdk.trace.data.DelegatingSpanData;
import io.opentelemetry.sdk.trace.data.SpanData;
import io.opentelemetry.sdk.trace.export.SpanExporter;
import java.util.ArrayList;
import java.util.Collection;
import java.util.Collections;
import java.util.List;
import java.util.Map;
import javax.annotation.Nonnull;

/**
 * Masks the payloads the instrumentations capture before the spans leave the process. The extension
 * moves HTTP bodies off the span into dash0_payload log records without masking them again, so this
 * is the only place they are masked.
 *
 * <p>Which rules apply to which attribute:
 *
 * <ul>
 *   <li>{@code http.request.body}: DASH0_MASK_REQUEST_BODY. JSON bodies are masked by key.
 *   <li>{@code http.response.body}: DASH0_MASK_RESPONSE_BODY. JSON bodies are masked by key.
 *   <li>{@code http.request.header.<name>}: DASH0_MASK_REQUEST_HEADERS, matched against the name.
 *   <li>{@code http.response.header.<name>}: DASH0_MASK_RESPONSE_HEADERS, matched against the name.
 *   <li>{@code url.query} and the query of {@code url.full}: DASH0_MASK_QUERY_PARAMS, matched
 *       against the parameter names.
 *   <li>Kafka and RabbitMQ {@code messaging.message.payload} and {@code messaging.message.headers}:
 *       DASH0_MASK_QUERY_PARAMS when sent, DASH0_MASK_RESPONSE_BODY when received, as in the
 *       Node.js distro. The payload is a string, so only {@code all} masks it; headers are matched
 *       by name.
 *   <li>The SQS {@code messaging.message.body} of a received message: DASH0_MASK_RESPONSE_BODY,
 *       {@code all} only.
 *   <li>Redis {@code db.statement} / {@code db.query.text}: DASH0_MASK_REQUEST_BODY, and {@code
 *       db.response.body}: DASH0_MASK_RESPONSE_BODY, {@code all} only.
 * </ul>
 */
public final class MaskingSpanExporter implements SpanExporter {
  static final String HTTP_REQUEST_BODY = "http.request.body";
  static final String HTTP_RESPONSE_BODY = "http.response.body";
  static final String HTTP_REQUEST_HEADER_PREFIX = "http.request.header.";
  static final String HTTP_RESPONSE_HEADER_PREFIX = "http.response.header.";
  static final String URL_QUERY = "url.query";
  static final String URL_FULL = "url.full";
  static final String MESSAGING_PAYLOAD = "messaging.message.payload";
  static final String MESSAGING_HEADERS = "messaging.message.headers";
  static final String SQS_MESSAGE_BODY = "messaging.message.body";
  static final String DB_STATEMENT = "db.statement";
  static final String DB_QUERY_TEXT = "db.query.text";
  static final String DB_RESPONSE_BODY = "db.response.body";
  static final AttributeKey<String> DB_SYSTEM = AttributeKey.stringKey("db.system");
  static final AttributeKey<String> DB_SYSTEM_NAME = AttributeKey.stringKey("db.system.name");

  private final SpanExporter delegate;
  private final MaskingConfig config;

  public MaskingSpanExporter(SpanExporter delegate, MaskingConfig config) {
    this.delegate = delegate;
    this.config = config;
  }

  @Override
  public CompletableResultCode export(@Nonnull Collection<SpanData> spans) {
    List<SpanData> masked = new ArrayList<>(spans.size());
    for (SpanData span : spans) {
      masked.add(mask(span));
    }
    return delegate.export(masked);
  }

  @Override
  public CompletableResultCode flush() {
    return delegate.flush();
  }

  @Override
  public CompletableResultCode shutdown() {
    return delegate.shutdown();
  }

  @Override
  public String toString() {
    return "MaskingSpanExporter{" + delegate + "}";
  }

  SpanData mask(SpanData span) {
    Attributes attributes = span.getAttributes();
    boolean redis =
        "redis".equals(attributes.get(DB_SYSTEM)) || "redis".equals(attributes.get(DB_SYSTEM_NAME));
    MaskingRules messagingRules =
        span.getKind() == SpanKind.PRODUCER ? config.queryParams : config.responseBody;

    AttributesBuilder builder = null;
    for (Map.Entry<AttributeKey<?>, Object> entry : attributes.asMap().entrySet()) {
      AttributeKey<?> key = entry.getKey();
      Object value = entry.getValue();
      Object masked = maskAttribute(key.getKey(), value, redis, messagingRules);
      if (masked != value) {
        if (builder == null) {
          builder = attributes.toBuilder();
        }
        put(builder, key, masked);
      }
    }
    return builder == null ? span : new MaskedSpanData(span, builder.build());
  }

  private Object maskAttribute(
      String name, Object value, boolean redis, MaskingRules messagingRules) {
    if (value instanceof String) {
      String string = (String) value;
      String masked = maskStringAttribute(name, string, redis, messagingRules);
      return masked.equals(string) ? value : masked;
    }
    if (value instanceof List) {
      @SuppressWarnings("unchecked")
      List<String> list = (List<String>) value;
      if (name.startsWith(HTTP_REQUEST_HEADER_PREFIX)) {
        return maskHeader(
            name.substring(HTTP_REQUEST_HEADER_PREFIX.length()), list, config.requestHeaders);
      }
      if (name.startsWith(HTTP_RESPONSE_HEADER_PREFIX)) {
        return maskHeader(
            name.substring(HTTP_RESPONSE_HEADER_PREFIX.length()), list, config.responseHeaders);
      }
      if (name.equals(MESSAGING_HEADERS)) {
        return maskMessagingHeaders(list, messagingRules);
      }
    }
    return value;
  }

  private String maskStringAttribute(
      String name, String value, boolean redis, MaskingRules messagingRules) {
    switch (name) {
      case HTTP_REQUEST_BODY:
        return PayloadMasker.maskJson(value, config.requestBody);
      case HTTP_RESPONSE_BODY:
        return PayloadMasker.maskJson(value, config.responseBody);
      case URL_QUERY:
        return PayloadMasker.maskQuery(value, config.queryParams);
      case URL_FULL:
        return PayloadMasker.maskUrl(value, config.queryParams);
      case MESSAGING_PAYLOAD:
        return PayloadMasker.maskString(value, messagingRules);
      case SQS_MESSAGE_BODY:
        return PayloadMasker.maskString(value, config.responseBody);
      case DB_STATEMENT:
      case DB_QUERY_TEXT:
        return redis ? PayloadMasker.maskString(value, config.requestBody) : value;
      case DB_RESPONSE_BODY:
        return redis ? PayloadMasker.maskString(value, config.responseBody) : value;
      default:
        break;
    }
    if (name.startsWith(HTTP_REQUEST_HEADER_PREFIX)) {
      String header = name.substring(HTTP_REQUEST_HEADER_PREFIX.length());
      return config.requestHeaders.matches(header) ? MASKED_VALUE : value;
    }
    if (name.startsWith(HTTP_RESPONSE_HEADER_PREFIX)) {
      String header = name.substring(HTTP_RESPONSE_HEADER_PREFIX.length());
      return config.responseHeaders.matches(header) ? MASKED_VALUE : value;
    }
    return value;
  }

  /** A header attribute holds all the values of one header; a matching name masks them all. */
  private static List<String> maskHeader(String header, List<String> values, MaskingRules rules) {
    return rules.matches(header) ? Collections.singletonList(MASKED_VALUE) : values;
  }

  /** Kafka headers are captured as {@code name=value} strings. */
  private static List<String> maskMessagingHeaders(List<String> headers, MaskingRules rules) {
    List<String> masked = null;
    for (int i = 0; i < headers.size(); i++) {
      String header = headers.get(i);
      int eq = header.indexOf('=');
      String name = eq >= 0 ? header.substring(0, eq) : header;
      if (rules.matches(name)) {
        if (masked == null) {
          masked = new ArrayList<>(headers);
        }
        masked.set(i, name + "=" + MASKED_VALUE);
      }
    }
    return masked == null ? headers : masked;
  }

  @SuppressWarnings("unchecked")
  private static void put(AttributesBuilder builder, AttributeKey<?> key, Object value) {
    if (key.getType() == AttributeType.STRING) {
      builder.put((AttributeKey<String>) key, (String) value);
    } else if (key.getType() == AttributeType.STRING_ARRAY) {
      builder.put((AttributeKey<List<String>>) key, (List<String>) value);
    }
  }

  private static final class MaskedSpanData extends DelegatingSpanData {
    private final Attributes attributes;

    MaskedSpanData(SpanData delegate, Attributes attributes) {
      super(delegate);
      this.attributes = attributes;
    }

    @Override
    public Attributes getAttributes() {
      return attributes;
    }
  }
}
