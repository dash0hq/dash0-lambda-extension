package io.dash0.javaagent.masking;

import com.sun.management.ThreadMXBean;
import io.opentelemetry.api.common.AttributeKey;
import io.opentelemetry.api.common.Attributes;
import io.opentelemetry.api.common.AttributesBuilder;
import io.opentelemetry.api.trace.Span;
import io.opentelemetry.api.trace.SpanKind;
import io.opentelemetry.sdk.autoconfigure.spi.internal.DefaultConfigProperties;
import io.opentelemetry.sdk.trace.ReadableSpan;
import io.opentelemetry.sdk.trace.SdkTracerProvider;
import io.opentelemetry.sdk.trace.SpanLimits;
import io.opentelemetry.sdk.trace.data.SpanData;
import io.opentelemetry.sdk.trace.export.SpanExporter;
import java.io.FileReader;
import java.lang.management.ManagementFactory;
import java.nio.charset.StandardCharsets;
import java.util.*;
import java.util.function.Supplier;
import java.util.regex.Matcher;
import java.util.regex.Pattern;

/** Throwaway benchmark for the distro-vs-extension masking comparison. Not committed. */
public class DistroBench {
  static Properties fx = new Properties();
  static final ThreadMXBean MX = (ThreadMXBean) ManagementFactory.getThreadMXBean();
  static long sink;

  static String get(String k) {
    return new String(Base64.getDecoder().decode(fx.getProperty(k)), StandardCharsets.UTF_8);
  }

  static Map<String, String> config(String mode) {
    // DASH0_MASK_RULES reaches the agent as the dash0.mask.rules config property.
    return mode.equals("custom")
        ? Collections.singletonMap("dash0.mask.rules", get("custom_rule"))
        : Collections.emptyMap();
  }

  static final Pattern HEADER = Pattern.compile("\"([^\"]+)\":\"([^\"]*)\"");

  /** The fixture span as the Java distro captures it: headers as one attribute per header. */
  static Attributes spanAttributes(String set) {
    AttributesBuilder b = Attributes.builder();
    String p = "span." + set + ".";
    for (String key : fx.stringPropertyNames()) {
      if (!key.startsWith(p)) continue;
      String name = key.substring(p.length());
      String value = get(key);
      if (name.equals("http.request.headers") || name.equals("http.response.headers")) {
        String prefix = name.equals("http.request.headers") ? "http.request.header." : "http.response.header.";
        Matcher m = HEADER.matcher(value);
        while (m.find()) b.put(AttributeKey.stringArrayKey(prefix + m.group(1)), Collections.singletonList(m.group(2)));
      } else {
        b.put(name, value);
      }
    }
    return b.build();
  }

  static SpanData span(Attributes a, int lengthLimit) {
    SdkTracerProvider tp =
        SdkTracerProvider.builder()
            .setSpanLimits(SpanLimits.builder().setMaxAttributeValueLength(lengthLimit).build())
            .build();
    Span s = tp.get("b").spanBuilder("POST").setSpanKind(SpanKind.CLIENT).setAllAttributes(a).startSpan();
    s.end();
    return ((ReadableSpan) s).toSpanData();
  }

  static List<SpanData> batch(String set, int n, int lengthLimit) {
    List<SpanData> l = new ArrayList<>();
    for (int i = 0; i < n; i++) l.add(span(spanAttributes(set), lengthLimit));
    return l;
  }

  /** Times batches of k calls; returns {p50 ns/op, p99 ns/op, bytes/op}. */
  static double[] measure(Runnable op) {
    for (int i = 0; i < 200_000; i++) op.run(); // warm-up
    long t0 = System.nanoTime();
    for (int i = 0; i < 1000; i++) op.run();
    double est = Math.max(1, (System.nanoTime() - t0) / 1000.0);
    int k = (int) Math.max(1, Math.min(10_000, 20_000 / est)); // ~20us per sample, above nanoTime granularity
    int samples = 5000;
    double[] per = new double[samples];
    long a0 = MX.getThreadAllocatedBytes(Thread.currentThread().getId());
    for (int s = 0; s < samples; s++) {
      long t = System.nanoTime();
      for (int i = 0; i < k; i++) op.run();
      per[s] = (System.nanoTime() - t) / (double) k;
    }
    long bytes = MX.getThreadAllocatedBytes(Thread.currentThread().getId()) - a0;
    Arrays.sort(per);
    return new double[] {per[samples / 2], per[samples * 99 / 100], bytes / (double) samples / k};
  }

  static void row(String name, Runnable op) {
    double[] r = measure(op);
    System.out.printf("| %-44s | %10.0f | %10.0f | %10.0f |%n", name, r[0], r[1], r[2]);
  }

  public static void main(String[] args) throws Exception {
    try (FileReader r = new FileReader(args[0])) {
      fx.load(r);
    }
    String what = args[1];
    String mode = args[2];
    int spans = Integer.parseInt(fx.getProperty("spans_per_invocation"));
    if (what.equals("cold")) {
      cold(mode, spans, args[3]);
    } else if (what.equals("warm")) {
      warm(mode, spans);
    } else {
      correctness(mode);
    }
  }

  static SpanExporter noop() {
    return SpanExporter.composite();
  }

  static void cold(String mode, int spans, String set) {
    // SDK and span construction first: in the agent they're loaded regardless of masking.
    List<SpanData> b = batch(set, spans, 1024);
    List<SpanData> b2 = batch(set, spans, 1024);
    SpanExporter delegate = noop();
    delegate.export(b);
    long tid = Thread.currentThread().getId();
    long a0 = MX.getThreadAllocatedBytes(tid);
    long t0 = System.nanoTime();
    MaskingSpanExporter ex =
        new MaskingSpanExporter(delegate, MaskingConfig.fromConfig(DefaultConfigProperties.createFromMap(config(mode))));
    long t1 = System.nanoTime();
    long a1 = MX.getThreadAllocatedBytes(tid);
    ex.export(b);
    long t2 = System.nanoTime();
    long a2 = MX.getThreadAllocatedBytes(tid);
    StringBuilder next = new StringBuilder();
    for (int i = 0; i < 10; i++) {
      long t = System.nanoTime();
      ex.export(i % 2 == 0 ? b2 : b);
      next.append(i == 0 ? "" : ",").append((System.nanoTime() - t) / 1000);
    }
    System.out.printf(
        "COLD %s %s: config+compile %d us (%d KB alloc), first export of %d spans %d us (%d KB alloc), next 10 exports us: [%s]%n",
        mode, set, (t1 - t0) / 1000, (a1 - a0) / 1024, spans, (t2 - t1) / 1000, (a2 - a1) / 1024, next);
  }

  static void warm(String mode, int spans) {
    MaskingConfig cfg = MaskingConfig.fromConfig(DefaultConfigProperties.createFromMap(config(mode)));
    MaskingSpanExporter ex = new MaskingSpanExporter(noop(), cfg);
    System.out.println("### " + mode);
    System.out.printf("| %-44s | %10s | %10s | %10s |%n", "case", "p50 ns", "p99 ns", "bytes/op");
    System.out.println("|---|---:|---:|---:|");
    for (String p : new String[] {"small_json", "json_4k", "truncated_json_4k", "form_encoded", "non_matching_json_4k", "plain_text"}) {
      String body = get("payload." + p);
      row("payload " + p + " (http.request.body, maskJson)", () -> sink += PayloadMasker.maskJson(body, cfg.requestBody).length());
    }
    String url = get("span.small_json.url.full");
    row("url.full query (maskUrl)", () -> sink += PayloadMasker.maskUrl(url, cfg.queryParams).length());
    for (String set : new String[] {"small_json", "json_4k"}) {
      for (int limit : new int[] {1024, Integer.MAX_VALUE}) {
        String l = limit == 1024 ? "distro limit 1024" : "no length limit";
        SpanData sd = span(spanAttributes(set), limit);
        row("span " + set + " (" + l + ")", () -> sink += ex.mask(sd).getAttributes().size());
        List<SpanData> b = batch(set, spans, limit);
        row("invocation " + spans + "x " + set + " (" + l + ")", () -> sink += ex.export(b).isSuccess() ? 1 : 0);
      }
    }
    if (sink == 42) System.out.println();
  }

  static void correctness(String mode) {
    MaskingConfig cfg = MaskingConfig.fromConfig(DefaultConfigProperties.createFromMap(config(mode)));
    for (String p : new String[] {"small_json", "json_4k", "truncated_json_4k", "form_encoded", "non_matching_json_4k", "plain_text"}) {
      String body = get("payload." + p);
      String m = PayloadMasker.maskJson(body, cfg.requestBody);
      System.out.printf("== %s %s changed=%s inLen=%d outLen=%d%n%s%n", mode, p, !m.equals(body), body.length(), m.length(), m.length() > 400 ? m.substring(0, 300) + " ... " + m.substring(m.length() - 100) : m);
    }
    for (String set : new String[] {"small_json", "json_4k"}) {
      for (int limit : new int[] {1024, Integer.MAX_VALUE}) {
        SpanData out = new MaskingSpanExporter(noop(), cfg).mask(span(spanAttributes(set), limit));
        System.out.println("== span " + mode + " " + set + " limit=" + limit);
        out.getAttributes().forEach((k, v) -> {
          String s = String.valueOf(v);
          System.out.println("  " + k.getKey() + " = " + (s.length() > 200 ? s.substring(0, 120) + " ... " + s.substring(s.length() - 60) + " [len " + s.length() + "]" : s));
        });
      }
    }
  }
}
