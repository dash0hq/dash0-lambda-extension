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

import io.opentelemetry.sdk.autoconfigure.spi.internal.DefaultConfigProperties;
import io.opentelemetry.sdk.trace.data.EventData;
import io.opentelemetry.sdk.trace.data.SpanData;
import io.opentelemetry.sdk.trace.export.SpanExporter;
import java.util.ArrayList;
import java.util.Collection;
import java.util.Collections;
import java.util.List;

/**
 * Checks that masking still covers what an instrumentation captures, without naming any attribute.
 *
 * <p>A test sends {@link #SECRET} through the real instrumentation, under {@link #FIELD} in a body,
 * query or message header, or as the value of the {@link #HEADER} HTTP header. It then passes the
 * spans it captured to {@link #assertMaskedByKey} or {@link #assertMaskedWithAll}. These mask the
 * spans with {@link MaskingSpanExporter} and fail if the secret is left in any attribute. When an
 * instrumentation or semconv upgrade moves a payload to a new attribute, the secret shows up there
 * and the test names it.
 */
public final class MaskingCanary {
  /** The value a test sends. No attribute may contain it after masking. */
  public static final String SECRET = "dash0-masking-canary";

  /** A body key, query parameter or message header name that the canary rules match. */
  public static final String FIELD = "masked_field";

  /**
   * An HTTP header name that the canary rules match. The test must capture it with {@code
   * otel.instrumentation.http.*.capture-*-headers}.
   */
  public static final String HEADER = "x-masked-field";

  private static final String BY_KEY_RULES = "[\".*masked.field.*\"]";
  private static final MaskingSpanExporter BY_KEY = exporter(BY_KEY_RULES);
  private static final MaskingSpanExporter ALL = exporter("all");

  private MaskingCanary() {}

  /** For payloads masked by key: JSON bodies, headers, query parameters, message headers. */
  public static void assertMaskedByKey(Collection<SpanData> spans) {
    assertMasked(spans, BY_KEY, "DASH0_MASK_RULES=" + BY_KEY_RULES);
  }

  /** For payloads that are strings, which only {@code all} masks: messages, Redis commands. */
  public static void assertMaskedWithAll(Collection<SpanData> spans) {
    assertMasked(spans, ALL, "DASH0_MASK_RULES=all");
  }

  private static void assertMasked(
      Collection<SpanData> spans, MaskingSpanExporter exporter, String setting) {
    if (find(spans).isEmpty()) {
      throw new AssertionError(
          "No attribute contains " + SECRET + ", so this test checks nothing. Spans: " + spans);
    }
    List<SpanData> masked = new ArrayList<>(spans.size());
    for (SpanData span : spans) {
      masked.add(exporter.mask(span));
    }
    List<String> leaks = find(masked);
    if (!leaks.isEmpty()) {
      throw new AssertionError(
          "With "
              + setting
              + ", "
              + SECRET
              + " is still in "
              + leaks
              + ". If an instrumentation now writes the payload under a new attribute, add it to"
              + " MaskingSpanExporter.");
    }
  }

  private static List<String> find(Collection<SpanData> spans) {
    List<String> found = new ArrayList<>();
    for (SpanData span : spans) {
      span.getAttributes()
          .forEach(
              (key, value) -> {
                if (String.valueOf(value).contains(SECRET)) {
                  found.add(span.getName() + ": " + key.getKey() + " = " + value);
                }
              });
      for (EventData event : span.getEvents()) {
        event
            .getAttributes()
            .forEach(
                (key, value) -> {
                  if (String.valueOf(value).contains(SECRET)) {
                    found.add(span.getName() + ", event " + event.getName() + ": " + key.getKey());
                  }
                });
      }
    }
    return found;
  }

  private static MaskingSpanExporter exporter(String rules) {
    return new MaskingSpanExporter(
        SpanExporter.composite(),
        MaskingConfig.fromConfig(
            DefaultConfigProperties.createFromMap(
                Collections.singletonMap(MaskingConfig.DASH0_MASK_RULES, rules))));
  }
}
