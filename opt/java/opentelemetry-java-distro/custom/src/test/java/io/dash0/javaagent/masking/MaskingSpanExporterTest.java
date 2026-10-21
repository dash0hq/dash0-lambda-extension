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

import static io.opentelemetry.api.common.AttributeKey.stringArrayKey;
import static io.opentelemetry.api.common.AttributeKey.stringKey;
import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertSame;

import io.opentelemetry.api.common.AttributeKey;
import io.opentelemetry.api.common.Attributes;
import io.opentelemetry.api.trace.Span;
import io.opentelemetry.api.trace.SpanKind;
import io.opentelemetry.sdk.autoconfigure.spi.internal.DefaultConfigProperties;
import io.opentelemetry.sdk.common.CompletableResultCode;
import io.opentelemetry.sdk.trace.SdkTracerProvider;
import io.opentelemetry.sdk.trace.data.SpanData;
import io.opentelemetry.sdk.trace.export.SimpleSpanProcessor;
import io.opentelemetry.sdk.trace.export.SpanExporter;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.Collection;
import java.util.Collections;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import org.junit.jupiter.api.Nested;
import org.junit.jupiter.api.Test;

/**
 * The cases of docs/masking.md, run against the attributes as the Java instrumentations record
 * them. The payloads are the ones the docs use.
 */
class MaskingSpanExporterTest {
  static final String DEFAULTS =
      "\".*pass.*\",\".*key.*\",\".*secret.*\",\".*credential.*\",\".*passphrase.*\",\".*token.*\",\".*signature.*\",\".*authorization.*\"";

  static final AttributeKey<String> REQUEST_BODY = stringKey("http.request.body");
  static final AttributeKey<String> RESPONSE_BODY = stringKey("http.response.body");
  static final AttributeKey<String> URL_FULL = stringKey("url.full");
  static final AttributeKey<String> URL_QUERY = stringKey("url.query");
  static final AttributeKey<String> MESSAGING_PAYLOAD = stringKey("messaging.message.payload");
  static final AttributeKey<List<String>> MESSAGING_HEADERS =
      stringArrayKey("messaging.message.headers");
  static final AttributeKey<String> SQS_MESSAGE_BODY = stringKey("messaging.message.body");
  static final AttributeKey<String> DB_SYSTEM = stringKey("db.system");
  static final AttributeKey<String> DB_STATEMENT = stringKey("db.statement");
  static final AttributeKey<String> DB_RESPONSE_BODY = stringKey("db.response.body");

  // http.request.body
  static final String HTTP_REQUEST_BODY =
      "{\"email\":\"jane@example.com\",\"password\":\"b-pw\",\"apiToken\":\"t\",\"orderId\":\"42\"}";
  // http.response.body
  static final String HTTP_RESPONSE_BODY =
      "{\"email\":\"jane@example.com\",\"password\":\"r-pw\",\"sessionToken\":\"s\",\"orderId\":\"42\"}";

  static final String MESSAGE_ATTRIBUTES =
      "\"MessageAttributes\":{"
          + "\"email\":{\"DataType\":\"String\",\"StringValue\":\"jane@example.com\"},"
          + "\"password\":{\"DataType\":\"String\",\"StringValue\":\"m-pw\"},"
          + "\"orderId\":{\"DataType\":\"String\",\"StringValue\":\"42\"}}";

  /** The request body of an SQS SendMessage, as the AWS SDK (2.21 and later) sends it. */
  static final String SQS_SEND_BODY =
      "{\"QueueUrl\":\"https://sqs.eu-central-1.amazonaws.com/123456789012/orders\","
          + "\"MessageBody\":\"order 42 for jane@example.com\","
          + MESSAGE_ATTRIBUTES
          + "}";

  /** The response body of an SQS ReceiveMessage. */
  static final String SQS_RECEIVE_BODY =
      "{\"Messages\":[{\"MessageId\":\"m1\",\"Body\":\"order 42 for jane@example.com\","
          + MESSAGE_ATTRIBUTES
          + "}]}";

  static final String URL =
      "https://api.example.com/orders?email=jane%40example.com&password=q-pw&orderId=42";
  static final String QUERY = "email=jane%40example.com&password=q-pw&orderId=42";

  static final String KAFKA_MESSAGE = "{\"email\":\"jane@example.com\",\"orderId\":42}";
  static final String REDIS_COMMAND = "SET user:1 jane@example.com";

  static AttributeKey<List<String>> requestHeader(String name) {
    return stringArrayKey("http.request.header." + name);
  }

  static AttributeKey<List<String>> responseHeader(String name) {
    return stringArrayKey("http.response.header." + name);
  }

  /** An HTTP client span with everything the HTTP instrumentations capture. */
  static Attributes httpClientAttributes() {
    return Attributes.builder()
        .put(URL_FULL, URL)
        .put(REQUEST_BODY, HTTP_REQUEST_BODY)
        .put(RESPONSE_BODY, HTTP_RESPONSE_BODY)
        .put(requestHeader("content-type"), "application/json")
        .put(requestHeader("x-email"), "jane@example.com")
        .put(requestHeader("x-password"), "h-pw")
        .put(requestHeader("authorization"), "Bearer abc")
        .put(requestHeader("x-signature"), "sig")
        .put(requestHeader("x-orderid"), "42")
        .put(responseHeader("content-type"), "application/json")
        .put(responseHeader("x-email"), "jane@example.com")
        .put(responseHeader("x-password"), "h-pw")
        .put(responseHeader("x-orderid"), "42")
        .build();
  }

  /** Exports one span through the masking exporter, configured from these env vars. */
  static Attributes export(Map<String, String> env, SpanKind kind, Attributes attributes) {
    Map<String, String> properties = new HashMap<>();
    // How the SDK maps DASH0_MASK_RULES onto dash0.mask.rules.
    env.forEach((name, value) -> properties.put(name.toLowerCase().replace('_', '.'), value));
    MaskingConfig config =
        MaskingConfig.fromConfig(DefaultConfigProperties.createFromMap(properties));

    CapturingExporter captured = new CapturingExporter();
    try (SdkTracerProvider tracerProvider =
        SdkTracerProvider.builder()
            .addSpanProcessor(SimpleSpanProcessor.create(new MaskingSpanExporter(captured, config)))
            .build()) {
      Span span = tracerProvider.get("test").spanBuilder("span").setSpanKind(kind).startSpan();
      span.setAllAttributes(attributes);
      span.end();
    }
    assertEquals(1, captured.spans.size());
    return captured.spans.get(0).getAttributes();
  }

  static Attributes exportHttp(Map<String, String> env) {
    return export(env, SpanKind.CLIENT, httpClientAttributes());
  }

  static Map<String, String> env(String... nameValuePairs) {
    Map<String, String> env = new HashMap<>();
    for (int i = 0; i < nameValuePairs.length; i += 2) {
      env.put(nameValuePairs[i], nameValuePairs[i + 1]);
    }
    return env;
  }

  static final class CapturingExporter implements SpanExporter {
    final List<SpanData> spans = new ArrayList<>();

    @Override
    public CompletableResultCode export(Collection<SpanData> spans) {
      this.spans.addAll(spans);
      return CompletableResultCode.ofSuccess();
    }

    @Override
    public CompletableResultCode flush() {
      return CompletableResultCode.ofSuccess();
    }

    @Override
    public CompletableResultCode shutdown() {
      return CompletableResultCode.ofSuccess();
    }
  }

  @Nested
  class Defaults {
    final Map<String, String> env = env();

    @Test
    void requestHeaders() {
      Attributes attributes = exportHttp(env);
      assertEquals(list("application/json"), attributes.get(requestHeader("content-type")));
      assertEquals(list("jane@example.com"), attributes.get(requestHeader("x-email")));
      assertEquals(list("****"), attributes.get(requestHeader("x-password")));
      assertEquals(list("****"), attributes.get(requestHeader("authorization")));
      assertEquals(list("****"), attributes.get(requestHeader("x-signature")));
      assertEquals(list("42"), attributes.get(requestHeader("x-orderid")));
    }

    @Test
    void requestBody() {
      assertEquals(
          "{\"email\":\"jane@example.com\",\"password\":\"****\",\"apiToken\":\"****\",\"orderId\":\"42\"}",
          exportHttp(env).get(REQUEST_BODY));
    }

    @Test
    void responseBody() {
      assertEquals(
          "{\"email\":\"jane@example.com\",\"password\":\"****\",\"sessionToken\":\"****\",\"orderId\":\"42\"}",
          exportHttp(env).get(RESPONSE_BODY));
    }

    @Test
    void responseHeaders() {
      Attributes attributes = exportHttp(env);
      assertEquals(list("jane@example.com"), attributes.get(responseHeader("x-email")));
      assertEquals(list("****"), attributes.get(responseHeader("x-password")));
      assertEquals(list("42"), attributes.get(responseHeader("x-orderid")));
    }

    @Test
    void queryOfTheUrl() {
      assertEquals(
          "https://api.example.com/orders?email=jane%40example.com&password=****&orderId=42",
          exportHttp(env).get(URL_FULL));
    }

    @Test
    void queryOfAServerRequest() {
      Attributes attributes = export(env, SpanKind.SERVER, Attributes.of(URL_QUERY, QUERY));
      assertEquals("email=jane%40example.com&password=****&orderId=42", attributes.get(URL_QUERY));
    }

    @Test
    void sqsSendMessage() {
      Attributes attributes =
          export(env, SpanKind.CLIENT, Attributes.of(REQUEST_BODY, SQS_SEND_BODY));
      assertEquals(
          "{\"QueueUrl\":\"https://sqs.eu-central-1.amazonaws.com/123456789012/orders\","
              + "\"MessageBody\":\"order 42 for jane@example.com\","
              + "\"MessageAttributes\":{"
              + "\"email\":{\"DataType\":\"String\",\"StringValue\":\"jane@example.com\"},"
              + "\"password\":\"****\","
              + "\"orderId\":{\"DataType\":\"String\",\"StringValue\":\"42\"}}}",
          attributes.get(REQUEST_BODY));
    }

    @Test
    void sqsReceiveMessage() {
      Attributes attributes =
          export(env, SpanKind.CLIENT, Attributes.of(RESPONSE_BODY, SQS_RECEIVE_BODY));
      assertEquals(
          "{\"Messages\":[{\"MessageId\":\"m1\",\"Body\":\"order 42 for jane@example.com\","
              + "\"MessageAttributes\":{"
              + "\"email\":{\"DataType\":\"String\",\"StringValue\":\"jane@example.com\"},"
              + "\"password\":\"****\","
              + "\"orderId\":{\"DataType\":\"String\",\"StringValue\":\"42\"}}}]}",
          attributes.get(RESPONSE_BODY));
    }

    @Test
    void spanWithoutPayloadsIsExportedAsIs() {
      Attributes attributes = Attributes.of(stringKey("http.request.method"), "POST");
      assertEquals(attributes.asMap(), export(env, SpanKind.CLIENT, attributes).asMap());
    }
  }

  @Nested
  class OwnRules {
    final Map<String, String> env = env("DASH0_MASK_RULES", "[\".*email.*\"]");

    @Test
    void requestBody() {
      assertEquals(
          "{\"email\":\"****\",\"password\":\"b-pw\",\"apiToken\":\"t\",\"orderId\":\"42\"}",
          exportHttp(env).get(REQUEST_BODY));
    }

    @Test
    void requestHeaders() {
      Attributes attributes = exportHttp(env);
      assertEquals(list("****"), attributes.get(requestHeader("x-email")));
      assertEquals(list("h-pw"), attributes.get(requestHeader("x-password")));
      assertEquals(list("Bearer abc"), attributes.get(requestHeader("authorization")));
      assertEquals(list("sig"), attributes.get(requestHeader("x-signature")));
    }

    @Test
    void query() {
      assertEquals(
          "https://api.example.com/orders?email=****&password=q-pw&orderId=42",
          exportHttp(env).get(URL_FULL));
    }
  }

  @Nested
  class RulesPlusDefaults {
    final Map<String, String> env = env("DASH0_MASK_RULES", "[" + DEFAULTS + ",\".*email.*\"]");

    @Test
    void requestBody() {
      assertEquals(
          "{\"email\":\"****\",\"password\":\"****\",\"apiToken\":\"****\",\"orderId\":\"42\"}",
          exportHttp(env).get(REQUEST_BODY));
    }

    @Test
    void sqsReceiveMessage() {
      Attributes attributes =
          export(env, SpanKind.CLIENT, Attributes.of(RESPONSE_BODY, SQS_RECEIVE_BODY));
      assertEquals(
          "{\"Messages\":[{\"MessageId\":\"m1\",\"Body\":\"order 42 for jane@example.com\","
              + "\"MessageAttributes\":{\"email\":\"****\",\"password\":\"****\","
              + "\"orderId\":{\"DataType\":\"String\",\"StringValue\":\"42\"}}}]}",
          attributes.get(RESPONSE_BODY));
    }
  }

  @Nested
  class RulesAll {
    final Map<String, String> env = env("DASH0_MASK_RULES", "all");

    @Test
    void everyPayloadIsMasked() {
      Attributes attributes = exportHttp(env);
      assertEquals("****", attributes.get(REQUEST_BODY));
      assertEquals("****", attributes.get(RESPONSE_BODY));
      assertEquals(list("****"), attributes.get(requestHeader("content-type")));
      assertEquals(list("****"), attributes.get(requestHeader("x-orderid")));
      assertEquals(list("****"), attributes.get(responseHeader("x-orderid")));
      assertEquals("https://api.example.com/orders?****", attributes.get(URL_FULL));
    }
  }

  @Nested
  class RequestBody {
    @Test
    void patterns() {
      Map<String, String> env = env("DASH0_MASK_REQUEST_BODY", "[\".*email.*\"]");
      Attributes attributes = exportHttp(env);
      assertEquals(
          "{\"email\":\"****\",\"password\":\"b-pw\",\"apiToken\":\"t\",\"orderId\":\"42\"}",
          attributes.get(REQUEST_BODY));
      // unchanged from the defaults
      assertEquals(
          "{\"email\":\"jane@example.com\",\"password\":\"****\",\"sessionToken\":\"****\",\"orderId\":\"42\"}",
          attributes.get(RESPONSE_BODY));
      assertEquals(list("****"), attributes.get(requestHeader("authorization")));
    }

    @Test
    void patternsOnSqsSendMessage() {
      Map<String, String> env = env("DASH0_MASK_REQUEST_BODY", "[\".*email.*\"]");
      Attributes attributes =
          export(env, SpanKind.CLIENT, Attributes.of(REQUEST_BODY, SQS_SEND_BODY));
      assertEquals(
          "{\"QueueUrl\":\"https://sqs.eu-central-1.amazonaws.com/123456789012/orders\","
              + "\"MessageBody\":\"order 42 for jane@example.com\","
              + "\"MessageAttributes\":{\"email\":\"****\","
              + "\"password\":{\"DataType\":\"String\",\"StringValue\":\"m-pw\"},"
              + "\"orderId\":{\"DataType\":\"String\",\"StringValue\":\"42\"}}}",
          attributes.get(REQUEST_BODY));
    }

    @Test
    void all() {
      Map<String, String> env = env("DASH0_MASK_REQUEST_BODY", "all");
      Attributes attributes = exportHttp(env);
      assertEquals("****", attributes.get(REQUEST_BODY));
      assertEquals(
          "{\"email\":\"jane@example.com\",\"password\":\"****\",\"sessionToken\":\"****\",\"orderId\":\"42\"}",
          attributes.get(RESPONSE_BODY));
      assertEquals(
          "****",
          export(env, SpanKind.CLIENT, Attributes.of(REQUEST_BODY, SQS_SEND_BODY))
              .get(REQUEST_BODY));
    }
  }

  @Nested
  class ResponseBody {
    @Test
    void patterns() {
      Map<String, String> env = env("DASH0_MASK_RESPONSE_BODY", "[\".*email.*\"]");
      assertEquals(
          "{\"email\":\"****\",\"password\":\"r-pw\",\"sessionToken\":\"s\",\"orderId\":\"42\"}",
          exportHttp(env).get(RESPONSE_BODY));
      Attributes sqs = export(env, SpanKind.CLIENT, Attributes.of(RESPONSE_BODY, SQS_RECEIVE_BODY));
      assertEquals(
          "{\"Messages\":[{\"MessageId\":\"m1\",\"Body\":\"order 42 for jane@example.com\","
              + "\"MessageAttributes\":{\"email\":\"****\","
              + "\"password\":{\"DataType\":\"String\",\"StringValue\":\"m-pw\"},"
              + "\"orderId\":{\"DataType\":\"String\",\"StringValue\":\"42\"}}}]}",
          sqs.get(RESPONSE_BODY));
    }

    @Test
    void all() {
      Map<String, String> env = env("DASH0_MASK_RESPONSE_BODY", "all");
      Attributes attributes = exportHttp(env);
      assertEquals("****", attributes.get(RESPONSE_BODY));
      assertEquals(
          "{\"email\":\"jane@example.com\",\"password\":\"****\",\"apiToken\":\"****\",\"orderId\":\"42\"}",
          attributes.get(REQUEST_BODY));
    }
  }

  @Nested
  class RequestHeaders {
    @Test
    void patterns() {
      Attributes attributes = exportHttp(env("DASH0_MASK_REQUEST_HEADERS", "[\".*email.*\"]"));
      assertEquals(list("application/json"), attributes.get(requestHeader("content-type")));
      assertEquals(list("****"), attributes.get(requestHeader("x-email")));
      assertEquals(list("h-pw"), attributes.get(requestHeader("x-password")));
      assertEquals(list("Bearer abc"), attributes.get(requestHeader("authorization")));
      assertEquals(list("sig"), attributes.get(requestHeader("x-signature")));
      assertEquals(list("42"), attributes.get(requestHeader("x-orderid")));
      // response headers keep the defaults
      assertEquals(list("****"), attributes.get(responseHeader("x-password")));
    }

    @Test
    void all() {
      Attributes attributes = exportHttp(env("DASH0_MASK_REQUEST_HEADERS", "all"));
      for (String header :
          Arrays.asList(
              "content-type",
              "x-email",
              "x-password",
              "authorization",
              "x-signature",
              "x-orderid")) {
        assertEquals(list("****"), attributes.get(requestHeader(header)), header);
      }
      assertEquals(list("application/json"), attributes.get(responseHeader("content-type")));
    }

    @Test
    void headerWithSeveralValuesIsMaskedAsOne() {
      Attributes attributes =
          export(
              env(),
              SpanKind.CLIENT,
              Attributes.of(requestHeader("x-api-key"), Arrays.asList("k1", "k2")));
      assertEquals(list("****"), attributes.get(requestHeader("x-api-key")));
    }
  }

  @Nested
  class ResponseHeaders {
    @Test
    void patterns() {
      Attributes attributes = exportHttp(env("DASH0_MASK_RESPONSE_HEADERS", "[\".*email.*\"]"));
      assertEquals(list("application/json"), attributes.get(responseHeader("content-type")));
      assertEquals(list("****"), attributes.get(responseHeader("x-email")));
      assertEquals(list("h-pw"), attributes.get(responseHeader("x-password")));
      assertEquals(list("42"), attributes.get(responseHeader("x-orderid")));
    }

    @Test
    void all() {
      Attributes attributes = exportHttp(env("DASH0_MASK_RESPONSE_HEADERS", "all"));
      assertEquals(list("****"), attributes.get(responseHeader("content-type")));
      assertEquals(list("****"), attributes.get(responseHeader("x-orderid")));
      assertEquals(list("application/json"), attributes.get(requestHeader("content-type")));
    }

    @Test
    void stringValuedHeaderAttribute() {
      // Apache HttpClient 5 records the decoded content encoding as a plain string.
      AttributeKey<String> key = stringKey("http.response.header.content_encoding");
      assertEquals(
          "****",
          export(
                  env("DASH0_MASK_RESPONSE_HEADERS", "all"),
                  SpanKind.CLIENT,
                  Attributes.of(key, "gzip"))
              .get(key));
      assertEquals("gzip", export(env(), SpanKind.CLIENT, Attributes.of(key, "gzip")).get(key));
    }
  }

  @Nested
  class QueryParams {
    @Test
    void patterns() {
      Map<String, String> env = env("DASH0_MASK_QUERY_PARAMS", "[\".*email.*\"]");
      assertEquals(
          "https://api.example.com/orders?email=****&password=q-pw&orderId=42",
          exportHttp(env).get(URL_FULL));
      assertEquals(
          "email=****&password=q-pw&orderId=42",
          export(env, SpanKind.SERVER, Attributes.of(URL_QUERY, QUERY)).get(URL_QUERY));
    }

    @Test
    void all() {
      Map<String, String> env = env("DASH0_MASK_QUERY_PARAMS", "all");
      assertEquals("https://api.example.com/orders?****", exportHttp(env).get(URL_FULL));
      assertEquals(
          "****", export(env, SpanKind.SERVER, Attributes.of(URL_QUERY, QUERY)).get(URL_QUERY));
    }

    @Test
    void fragmentAndUrlWithoutQueryAreKept() {
      Map<String, String> env = env("DASH0_MASK_QUERY_PARAMS", "all");
      assertEquals(
          "https://api.example.com/orders?****#top",
          export(
                  env,
                  SpanKind.CLIENT,
                  Attributes.of(URL_FULL, "https://api.example.com/orders?a=1#top"))
              .get(URL_FULL));
      assertEquals(
          "https://api.example.com/orders",
          export(env, SpanKind.CLIENT, Attributes.of(URL_FULL, "https://api.example.com/orders"))
              .get(URL_FULL));
    }

    @Test
    void parameterNamesAreDecoded() {
      Map<String, String> env = env("DASH0_MASK_QUERY_PARAMS", "[\"^api key$\"]");
      assertEquals(
          "api+key=****&b=2",
          export(env, SpanKind.SERVER, Attributes.of(URL_QUERY, "api+key=s&b=2")).get(URL_QUERY));
    }
  }

  @Nested
  class CombiningVariables {
    final Map<String, String> env =
        env("DASH0_MASK_RULES", "[\".*orderId.*\"]", "DASH0_MASK_REQUEST_BODY", "[\".*email.*\"]");

    @Test
    void perPayloadVariableWinsForItsPayload() {
      assertEquals(
          "{\"email\":\"****\",\"password\":\"b-pw\",\"apiToken\":\"t\",\"orderId\":\"42\"}",
          exportHttp(env).get(REQUEST_BODY));
    }

    @Test
    void rulesApplyToTheRest() {
      Attributes attributes = exportHttp(env);
      assertEquals(
          "{\"email\":\"jane@example.com\",\"password\":\"r-pw\",\"sessionToken\":\"s\",\"orderId\":\"****\"}",
          attributes.get(RESPONSE_BODY));
      assertEquals(list("****"), attributes.get(requestHeader("x-orderid")));
      assertEquals(list("h-pw"), attributes.get(requestHeader("x-password")));
      assertEquals(list("Bearer abc"), attributes.get(requestHeader("authorization")));
      assertEquals(
          "https://api.example.com/orders?email=jane%40example.com&password=q-pw&orderId=****",
          attributes.get(URL_FULL));
    }
  }

  @Nested
  class InvalidValues {
    @Test
    void invalidRulesFallBackToTheDefaults() {
      assertEquals(
          "{\"email\":\"jane@example.com\",\"password\":\"****\",\"apiToken\":\"****\",\"orderId\":\"42\"}",
          exportHttp(env("DASH0_MASK_RULES", ".*email.*")).get(REQUEST_BODY));
    }

    @Test
    void invalidPerPayloadValueFallsBackToTheRules() {
      Map<String, String> env =
          env("DASH0_MASK_RULES", "[\".*email.*\"]", "DASH0_MASK_REQUEST_BODY", "[\".*email.*\",]");
      assertEquals(
          "{\"email\":\"****\",\"password\":\"b-pw\",\"apiToken\":\"t\",\"orderId\":\"42\"}",
          exportHttp(env).get(REQUEST_BODY));
    }

    @Test
    void emptyListMasksNothing() {
      Attributes attributes = exportHttp(env("DASH0_MASK_RULES", "[]"));
      assertEquals(HTTP_REQUEST_BODY, attributes.get(REQUEST_BODY));
      assertEquals(list("Bearer abc"), attributes.get(requestHeader("authorization")));
    }
  }

  @Nested
  class StringPayloads {
    @Test
    void nonJsonBodyIsNotParsed() {
      String form = "Action=SendMessage&MessageBody=hi&password=b-pw";
      assertEquals(
          form,
          export(env(), SpanKind.CLIENT, Attributes.of(REQUEST_BODY, form)).get(REQUEST_BODY));
      assertEquals(
          "****",
          export(
                  env("DASH0_MASK_REQUEST_BODY", "all"),
                  SpanKind.CLIENT,
                  Attributes.of(REQUEST_BODY, form))
              .get(REQUEST_BODY));
    }

    @Test
    void sqsMessageBodyKeyCanBeMasked() {
      Map<String, String> env =
          env("DASH0_MASK_RULES", "[" + DEFAULTS + ",\".*MessageBody.*\",\"^Body$\"]");
      assertEquals(
          "{\"QueueUrl\":\"https://sqs.eu-central-1.amazonaws.com/123456789012/orders\","
              + "\"MessageBody\":\"****\","
              + "\"MessageAttributes\":{"
              + "\"email\":{\"DataType\":\"String\",\"StringValue\":\"jane@example.com\"},"
              + "\"password\":\"****\","
              + "\"orderId\":{\"DataType\":\"String\",\"StringValue\":\"42\"}}}",
          export(env, SpanKind.CLIENT, Attributes.of(REQUEST_BODY, SQS_SEND_BODY))
              .get(REQUEST_BODY));
      assertEquals(
          "{\"Messages\":[{\"MessageId\":\"m1\",\"Body\":\"****\","
              + "\"MessageAttributes\":{"
              + "\"email\":{\"DataType\":\"String\",\"StringValue\":\"jane@example.com\"},"
              + "\"password\":\"****\","
              + "\"orderId\":{\"DataType\":\"String\",\"StringValue\":\"42\"}}}]}",
          export(env, SpanKind.CLIENT, Attributes.of(RESPONSE_BODY, SQS_RECEIVE_BODY))
              .get(RESPONSE_BODY));
    }

    @Test
    void sqsReceivedMessageIsMaskedWithAllOnly() {
      String message = "Message(MessageId=m1, Body=order 42 for jane@example.com)";
      Attributes attributes = Attributes.of(SQS_MESSAGE_BODY, message);
      assertEquals(
          message,
          export(env("DASH0_MASK_RULES", "[\".*Body.*\"]"), SpanKind.CONSUMER, attributes)
              .get(SQS_MESSAGE_BODY));
      assertEquals(
          "****",
          export(env("DASH0_MASK_RESPONSE_BODY", "all"), SpanKind.CONSUMER, attributes)
              .get(SQS_MESSAGE_BODY));
    }

    @Test
    void kafkaMessageIsNotParsed() {
      Attributes attributes = Attributes.of(MESSAGING_PAYLOAD, KAFKA_MESSAGE);
      assertEquals(
          KAFKA_MESSAGE,
          export(env("DASH0_MASK_RULES", "[\".*email.*\"]"), SpanKind.PRODUCER, attributes)
              .get(MESSAGING_PAYLOAD));
    }

    @Test
    void sentMessagesFollowTheQueryParamRules() {
      Attributes attributes = Attributes.of(MESSAGING_PAYLOAD, KAFKA_MESSAGE);
      assertEquals(
          "****",
          export(env("DASH0_MASK_QUERY_PARAMS", "all"), SpanKind.PRODUCER, attributes)
              .get(MESSAGING_PAYLOAD));
      assertEquals(
          KAFKA_MESSAGE,
          export(env("DASH0_MASK_RESPONSE_BODY", "all"), SpanKind.PRODUCER, attributes)
              .get(MESSAGING_PAYLOAD));
    }

    @Test
    void receivedMessagesFollowTheResponseBodyRules() {
      Attributes attributes = Attributes.of(MESSAGING_PAYLOAD, KAFKA_MESSAGE);
      assertEquals(
          "****",
          export(env("DASH0_MASK_RESPONSE_BODY", "all"), SpanKind.CONSUMER, attributes)
              .get(MESSAGING_PAYLOAD));
      // RabbitMQ basicGet spans are CLIENT spans
      assertEquals(
          "****",
          export(env("DASH0_MASK_RESPONSE_BODY", "all"), SpanKind.CLIENT, attributes)
              .get(MESSAGING_PAYLOAD));
      assertEquals(
          KAFKA_MESSAGE,
          export(env("DASH0_MASK_QUERY_PARAMS", "all"), SpanKind.CONSUMER, attributes)
              .get(MESSAGING_PAYLOAD));
    }

    @Test
    void kafkaHeadersAreMaskedByName() {
      Attributes attributes =
          Attributes.of(
              MESSAGING_HEADERS, Arrays.asList("traceparent=[48, 48]", "x-api-key=[107, 49]"));
      assertEquals(
          Arrays.asList("traceparent=[48, 48]", "x-api-key=****"),
          export(env(), SpanKind.PRODUCER, attributes).get(MESSAGING_HEADERS));
      assertEquals(
          Arrays.asList("traceparent=****", "x-api-key=****"),
          export(env("DASH0_MASK_QUERY_PARAMS", "all"), SpanKind.PRODUCER, attributes)
              .get(MESSAGING_HEADERS));
    }

    @Test
    void redisCommandsAndRepliesAreMaskedWithAllOnly() {
      Attributes attributes =
          Attributes.builder()
              .put(DB_SYSTEM, "redis")
              .put(DB_STATEMENT, REDIS_COMMAND)
              .put(DB_RESPONSE_BODY, "OK")
              .build();
      Attributes rules =
          export(env("DASH0_MASK_RULES", "[\".*email.*\"]"), SpanKind.CLIENT, attributes);
      assertEquals(REDIS_COMMAND, rules.get(DB_STATEMENT));

      Attributes requestAll =
          export(env("DASH0_MASK_REQUEST_BODY", "all"), SpanKind.CLIENT, attributes);
      assertEquals("****", requestAll.get(DB_STATEMENT));
      assertEquals("OK", requestAll.get(DB_RESPONSE_BODY));

      Attributes responseAll =
          export(env("DASH0_MASK_RESPONSE_BODY", "all"), SpanKind.CLIENT, attributes);
      assertEquals(REDIS_COMMAND, responseAll.get(DB_STATEMENT));
      assertEquals("****", responseAll.get(DB_RESPONSE_BODY));
    }

    @Test
    void otherDatabaseStatementsAreNotMasked() {
      Attributes attributes =
          Attributes.builder()
              .put(DB_SYSTEM, "postgresql")
              .put(DB_STATEMENT, "SELECT * FROM users WHERE email = ?")
              .build();
      assertEquals(
          "SELECT * FROM users WHERE email = ?",
          export(env("DASH0_MASK_RULES", "all"), SpanKind.CLIENT, attributes).get(DB_STATEMENT));
    }
  }

  @Nested
  class JsonBodies {
    @Test
    void nestedKeysAndArraysAreMasked() {
      String body =
          "{\"user\":{\"name\":\"jane\",\"credentials\":{\"a\":1}},\"items\":[{\"token\":\"t\",\"n\":1.50}],\"keys\":[1,2]}";
      assertEquals(
          "{\"user\":{\"name\":\"jane\",\"credentials\":\"****\"},\"items\":[{\"token\":\"****\",\"n\":1.50}],\"keys\":\"****\"}",
          export(env(), SpanKind.CLIENT, Attributes.of(REQUEST_BODY, body)).get(REQUEST_BODY));
    }

    @Test
    void bodyWithoutMatchingKeysKeepsItsFormatting() {
      String body = "{ \"email\": \"jane@example.com\",\n  \"orderId\": 42 }";
      assertEquals(
          body,
          export(env(), SpanKind.CLIENT, Attributes.of(REQUEST_BODY, body)).get(REQUEST_BODY));
    }

    @Test
    void truncatedBodyIsMaskedUpToTheCut() {
      // Bodies longer than otel.span.attribute.value.length.limit arrive cut off.
      assertEquals(
          "{\"email\":\"jane@example.com\",\"password\":\"****\",\"items\":[{\"a\":1},{}]}",
          export(
                  env(),
                  SpanKind.CLIENT,
                  Attributes.of(
                      REQUEST_BODY,
                      "{\"email\":\"jane@example.com\",\"password\":\"b-pw\",\"items\":[{\"a\":1},{\"b\":\"lo"))
              .get(REQUEST_BODY));
      assertEquals(
          "{\"email\":\"jane@example.com\",\"password\":\"****\"}",
          export(
                  env(),
                  SpanKind.CLIENT,
                  Attributes.of(REQUEST_BODY, "{\"email\":\"jane@example.com\",\"password\":\"b-p"))
              .get(REQUEST_BODY));
    }

    @Test
    void truncatedBodyWithoutMatchingKeysIsKept() {
      String body = "{\"email\":\"jane@example.com\",\"note\":\"lo";
      assertEquals(
          body,
          export(env(), SpanKind.CLIENT, Attributes.of(REQUEST_BODY, body)).get(REQUEST_BODY));
    }

    @Test
    void scalarJsonIsNotParsed() {
      String body = "\"password\"";
      assertEquals(
          body,
          export(env(), SpanKind.CLIENT, Attributes.of(REQUEST_BODY, body)).get(REQUEST_BODY));
    }
  }

  @Test
  void unmaskedSpanIsPassedThrough() {
    MaskingSpanExporter exporter =
        new MaskingSpanExporter(
            new CapturingExporter(),
            MaskingConfig.fromConfig(
                DefaultConfigProperties.createFromMap(Collections.emptyMap())));
    try (SdkTracerProvider tracerProvider = SdkTracerProvider.builder().build()) {
      Span span = tracerProvider.get("test").spanBuilder("span").startSpan();
      span.setAttribute("http.request.method", "GET");
      span.end();
      SpanData data = ((io.opentelemetry.sdk.trace.ReadableSpan) span).toSpanData();
      assertSame(data, exporter.mask(data));
    }
  }

  private static List<String> list(String value) {
    return Collections.singletonList(value);
  }
}
