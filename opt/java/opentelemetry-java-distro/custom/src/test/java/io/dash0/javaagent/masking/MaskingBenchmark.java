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

import com.sun.management.ThreadMXBean;
import io.opentelemetry.api.common.AttributeKey;
import io.opentelemetry.api.common.Attributes;
import io.opentelemetry.api.trace.Span;
import io.opentelemetry.api.trace.SpanKind;
import io.opentelemetry.sdk.autoconfigure.spi.internal.DefaultConfigProperties;
import io.opentelemetry.sdk.trace.ReadableSpan;
import io.opentelemetry.sdk.trace.SdkTracerProvider;
import io.opentelemetry.sdk.trace.data.SpanData;
import io.opentelemetry.sdk.trace.export.SpanExporter;
import java.lang.management.ManagementFactory;
import java.util.*;

/**
 * Measures what the masking exporter costs per span. Not a JUnit test; run it with {@code ./gradlew
 * :custom:maskingBenchmark}. The numbers are rough: a plain timing loop after a warm-up, best of
 * five rounds.
 */
public class MaskingBenchmark {
  static SdkTracerProvider tp = SdkTracerProvider.builder().build();

  static SpanData span(SpanKind kind, Attributes a) {
    Span s = tp.get("b").spanBuilder("s").setSpanKind(kind).setAllAttributes(a).startSpan();
    s.end();
    return ((ReadableSpan) s).toSpanData();
  }

  static Attributes common() {
    return Attributes.builder()
        .put("http.request.method", "POST")
        .put("http.response.status_code", 200L)
        .put("server.address", "api.example.com")
        .put("server.port", 443L)
        .put("network.protocol.version", "1.1")
        .put("thread.id", 1L)
        .put("thread.name", "main")
        .put("url.full", "https://api.example.com/orders")
        .build();
  }

  static String json(int targetLen, boolean withSecret) {
    StringBuilder sb = new StringBuilder("{");
    int i = 0;
    while (sb.length() < targetLen - 40) {
      if (i > 0) sb.append(',');
      sb.append("\"field")
          .append(i)
          .append("\":{\"name\":\"value ")
          .append(i)
          .append("\",\"n\":")
          .append(i)
          .append('}');
      i++;
    }
    if (withSecret) sb.append(",\"password\":\"hunter2\"");
    return sb.append('}').toString();
  }

  public static void main(String[] args) {
    SpanExporter noop = SpanExporter.composite();
    MaskingSpanExporter ex =
        new MaskingSpanExporter(
            noop,
            MaskingConfig.fromConfig(
                DefaultConfigProperties.createFromMap(Collections.emptyMap())));
    MaskingSpanExporter exAll =
        new MaskingSpanExporter(
            noop,
            MaskingConfig.fromConfig(
                DefaultConfigProperties.createFromMap(
                    Collections.singletonMap("dash0.mask.rules", "all"))));

    AttributeKey<String> req = AttributeKey.stringKey("http.request.body");
    AttributeKey<String> res = AttributeKey.stringKey("http.response.body");
    Map<String, SpanData> cases = new LinkedHashMap<>();
    cases.put("no payload attributes (8 attrs)", span(SpanKind.INTERNAL, common()));
    cases.put(
        "200 B JSON req body, 1 match",
        span(SpanKind.CLIENT, common().toBuilder().put(req, json(200, true)).build()));
    cases.put(
        "1 KB JSON req+res bodies, 1 match each",
        span(
            SpanKind.CLIENT,
            common().toBuilder().put(req, json(1024, true)).put(res, json(1024, true)).build()));
    cases.put(
        "1 KB JSON req+res bodies, no match",
        span(
            SpanKind.CLIENT,
            common().toBuilder().put(req, json(1024, false)).put(res, json(1024, false)).build()));
    cases.put(
        "1 KB non-JSON body",
        span(
            SpanKind.CLIENT,
            common().toBuilder()
                .put(req, "Action=SendMessage&" + new String(new char[1000]).replace('\0', 'x'))
                .build()));
    cases.put(
        "url.full with query, 1 match",
        span(
            SpanKind.CLIENT,
            common().toBuilder()
                .put(
                    "url.full",
                    "https://api.example.com/orders?email=a%40b.c&password=x&orderId=42&page=2")
                .build()));
    cases.put(
        "5 captured headers, 1 match",
        span(
            SpanKind.CLIENT,
            common().toBuilder()
                .put(
                    AttributeKey.stringArrayKey("http.request.header.content-type"),
                    Collections.singletonList("application/json"))
                .put(
                    AttributeKey.stringArrayKey("http.request.header.authorization"),
                    Collections.singletonList("Bearer abc"))
                .put(
                    AttributeKey.stringArrayKey("http.request.header.accept"),
                    Collections.singletonList("*/*"))
                .put(
                    AttributeKey.stringArrayKey("http.request.header.x-request-id"),
                    Collections.singletonList("1"))
                .put(
                    AttributeKey.stringArrayKey("http.request.header.user-agent"),
                    Collections.singletonList("java"))
                .build()));

    ThreadMXBean mx = (ThreadMXBean) ManagementFactory.getThreadMXBean();
    long tid = Thread.currentThread().getId();
    System.out.printf(
        "%-42s %12s %12s %12s%n",
        "case (defaults unless noted)", "ns/span", "bytes/span", "all: ns/span");
    for (Map.Entry<String, SpanData> c : cases.entrySet()) {
      SpanData sd = c.getValue();
      long sink = 0;
      for (int i = 0; i < 200_000; i++) {
        sink += ex.mask(sd).hashCode();
        sink += exAll.mask(sd).hashCode();
      }
      int n = 500_000;
      long best = Long.MAX_VALUE, bestAll = Long.MAX_VALUE, bytes = 0;
      for (int r = 0; r < 5; r++) {
        long a0 = mx.getThreadAllocatedBytes(tid);
        long t0 = System.nanoTime();
        for (int i = 0; i < n; i++) sink += ex.mask(sd).getAttributes().size();
        long t1 = System.nanoTime();
        bytes = (mx.getThreadAllocatedBytes(tid) - a0) / n;
        best = Math.min(best, (t1 - t0) / n);
        long t2 = System.nanoTime();
        for (int i = 0; i < n; i++) sink += exAll.mask(sd).getAttributes().size();
        bestAll = Math.min(bestAll, (System.nanoTime() - t2) / n);
      }
      System.out.printf("%-42s %12d %12d %12d%n", c.getKey(), best, bytes, bestAll);
      if (sink == 42) System.out.println();
    }
  }
}
