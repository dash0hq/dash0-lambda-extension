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
import java.util.HashMap;
import java.util.Map;
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
}
