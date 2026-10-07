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
package io.dash0.javaagent;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;
import static org.mockito.Mockito.mock;
import static org.mockito.Mockito.when;

import io.opentelemetry.sdk.autoconfigure.spi.ConfigProperties;
import io.opentelemetry.sdk.common.CompletableResultCode;
import io.opentelemetry.sdk.metrics.InstrumentType;
import io.opentelemetry.sdk.metrics.SdkMeterProvider;
import io.opentelemetry.sdk.metrics.SdkMeterProviderBuilder;
import io.opentelemetry.sdk.metrics.data.AggregationTemporality;
import io.opentelemetry.sdk.metrics.data.MetricData;
import io.opentelemetry.sdk.metrics.export.MetricExporter;
import io.opentelemetry.sdk.metrics.export.PeriodicMetricReader;
import java.util.ArrayList;
import java.util.Collection;
import java.util.HashMap;
import java.util.HashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.params.ParameterizedTest;
import org.junit.jupiter.params.provider.ValueSource;

public class Dash0ConfiguratorTest {
  @Test
  void testStripTraceSuffixWhenNotPresent() {
    String result = Dash0Configurator.stripTracesSuffix(Dash0Configurator.DASH0_EXTENSION_ENDPOINT_URL);
    assertEquals(Dash0Configurator.DASH0_EXTENSION_ENDPOINT_URL, result);
  }

  @Test
  void testStripTraceSuffixWhenPresent() {
    String result =
        Dash0Configurator.stripTracesSuffix(Dash0Configurator.DASH0_EXTENSION_ENDPOINT_URL + "/v1/traces");
    assertEquals(Dash0Configurator.DASH0_EXTENSION_ENDPOINT_URL, result);
  }

  private static ConfigProperties configOf(Map<String, String> values) {
    ConfigProperties cfg = mock(ConfigProperties.class);
    when(cfg.getString(org.mockito.ArgumentMatchers.anyString()))
        .thenAnswer(invocation -> values.get(invocation.<String>getArgument(0)));
    return cfg;
  }

  @Test
  void metricsExporterDefaultsToOtlp() {
    Map<String, String> result = new Dash0Configurator().propertiesCustomizer(configOf(new HashMap<>()));
    assertEquals("otlp", result.get("otel.metrics.exporter"));
  }

  @ParameterizedTest
  @ValueSource(strings = {"none", "console", "logging", "otlp"})
  void explicitMetricsExporterIsNotOverridden(String exporter) {
    Map<String, String> values = new HashMap<>();
    values.put("otel.metrics.exporter", exporter);
    Map<String, String> result = new Dash0Configurator().propertiesCustomizer(configOf(values));
    assertFalse(result.containsKey("otel.metrics.exporter"));
  }

  @Test
  void otherExportSettingsAreUnchanged() {
    Map<String, String> values = new HashMap<>();
    values.put(Dash0Configurator.DASH0_TOKEN, "secret");
    Map<String, String> result = new Dash0Configurator().propertiesCustomizer(configOf(values));
    assertEquals(Dash0Configurator.DASH0_EXTENSION_ENDPOINT_URL, result.get("otel.exporter.otlp.endpoint"));
    assertEquals("http/protobuf", result.get("otel.exporter.otlp.protocol"));
    assertTrue(result.get("otel.exporter.otlp.headers").contains("Authorization=Bearer secret"));
    assertFalse(result.containsKey("otel.traces.exporter"));
    assertFalse(result.containsKey("otel.logs.exporter"));
  }

  private static final String RUNTIME_TELEMETRY = "otel.instrumentation.runtime-telemetry.enabled";

  @Test
  void runtimeTelemetryIsDisabledByDefault() {
    Map<String, String> result = new Dash0Configurator().propertiesCustomizer(configOf(new HashMap<>()));
    assertEquals("false", result.get(RUNTIME_TELEMETRY));
  }

  @Test
  void explicitRuntimeTelemetryIsNotOverridden() {
    Map<String, String> values = new HashMap<>();
    values.put(RUNTIME_TELEMETRY, "true");
    Map<String, String> result = new Dash0Configurator().propertiesCustomizer(configOf(values));
    assertFalse(result.containsKey(RUNTIME_TELEMETRY));
  }

  /** Records one counter per scope through a meter provider built by the configurator. */
  private static Set<String> exportedMetricNames(Map<String, String> values) {
    List<MetricData> exported = new ArrayList<>();
    MetricExporter exporter =
        new MetricExporter() {
          @Override
          public AggregationTemporality getAggregationTemporality(InstrumentType type) {
            return AggregationTemporality.CUMULATIVE;
          }

          @Override
          public CompletableResultCode export(Collection<MetricData> metrics) {
            exported.addAll(metrics);
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
        };
    SdkMeterProviderBuilder builder =
        SdkMeterProvider.builder().registerMetricReader(PeriodicMetricReader.create(exporter));
    SdkMeterProvider provider =
        new Dash0Configurator().meterProviderCustomizer(builder, configOf(values)).build();
    provider.get("custom-app").counterBuilder("orders.processed").build().add(1);
    provider.get("io.opentelemetry.sdk.trace").counterBuilder("processedSpans").build().add(1);
    provider.get("io.opentelemetry.exporters.otlp-http").counterBuilder("otlp.exporter.seen").build().add(1);
    provider.get("io.opentelemetry.runtime-telemetry-java8").counterBuilder("jvm.class.loaded").build().add(1);
    provider.forceFlush().join(5, java.util.concurrent.TimeUnit.SECONDS);
    provider.close();
    Set<String> names = new HashSet<>();
    for (MetricData m : exported) {
      names.add(m.getName());
    }
    return names;
  }

  @Test
  void sdkSelfMetricsAreDropped() {
    Set<String> names = exportedMetricNames(new HashMap<>());
    assertTrue(names.contains("orders.processed"));
    assertTrue(names.contains("jvm.class.loaded"));
    assertFalse(names.contains("processedSpans"));
    assertFalse(names.contains("otlp.exporter.seen"));
  }

}
