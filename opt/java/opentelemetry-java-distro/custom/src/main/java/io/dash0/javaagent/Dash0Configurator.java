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

import com.google.auto.service.AutoService;
import io.dash0.javaagent.utils.Strings;
import io.opentelemetry.sdk.autoconfigure.spi.AutoConfigurationCustomizer;
import io.opentelemetry.sdk.autoconfigure.spi.AutoConfigurationCustomizerProvider;
import io.opentelemetry.sdk.autoconfigure.spi.ConfigProperties;
import io.opentelemetry.sdk.trace.SdkTracerProviderBuilder;
import io.opentelemetry.sdk.trace.export.SimpleSpanProcessor;
import java.util.*;
import java.util.logging.Logger;

/**
 * This is one of the main entry points for Instrumentation Agent's customizations. It allows
 * configuring the {@link AutoConfigurationCustomizer}. See the {@link
 * #customize(AutoConfigurationCustomizer)} method below.
 *
 * <p>Also see {@link <a
 * href="https://github.com/open-telemetry/opentelemetry-java/issues/2022">OpenTelemetry Java
 * 'Confusing configuration story' Issue #2022</a>}
 *
 * @see AutoConfigurationCustomizerProvider
 */
@AutoService(AutoConfigurationCustomizerProvider.class)
public class Dash0Configurator implements AutoConfigurationCustomizerProvider {
  public static final String DASH0_TOKEN = "dash0.token";
  public static final String DASH0_EXTENSION_ENDPOINT = "dash0.extension.endpoint";
  public static final String DASH0_DEBUG_SPANDUMP = "dash0.debug.spandump";

  public static final Logger LOGGER = Logger.getLogger(Dash0Configurator.class.getName());

  public static final String DASH0_EXTENSION_ENDPOINT_URL =
      "http://127.0.0.1:9009";

  @Override
  public void customize(AutoConfigurationCustomizer autoConfiguration) {
    autoConfiguration
        .addPropertiesCustomizer(this::propertiesCustomizer)
        .addTracerProviderCustomizer(this::tracerProviderCustomizer)
        .addPropertiesSupplier(this::getDefaultProperties);
  }

  private SdkTracerProviderBuilder tracerProviderCustomizer(
      SdkTracerProviderBuilder tracerProvider, ConfigProperties cfg) {
    String debugSpanDump = cfg.getString(DASH0_DEBUG_SPANDUMP);
    if (!Strings.isBlank(debugSpanDump)) {
      if (!(debugSpanDump.split("/").length > 1)) {
        LOGGER.warning("Spandump path '" + debugSpanDump + "' is not valid; spandump is disabled.");
      } else {
        try {
          tracerProvider.addSpanProcessor(
              SimpleSpanProcessor.create(FileSpanExporter.create(debugSpanDump)));

          LOGGER.finest("Dumping spans to '" + debugSpanDump + "' file");
        } catch (Exception e) {
          LOGGER.severe("Cannot create spandump exporter to '" + debugSpanDump + "' file: " + e);
        }
      }
    }

    return tracerProvider;
  }

  private Map<String, String> propertiesCustomizer(ConfigProperties originalCfg) {
    String accessToken = originalCfg.getString(DASH0_TOKEN);
    if (Strings.isBlank(accessToken)) {
      LOGGER.warning(
          "Dash0 token not provided (env var 'DASH0_TOKEN' not set); no data will be sent.");
      return Collections.emptyMap();
    }

    Map<String, String> customizedCfg = new HashMap<>();

    List<String> headers = new ArrayList<>();

    String rawHeaders = originalCfg.getString("otel.exporter.otlp.headers");
    if (!Strings.isBlank(rawHeaders)) {
      headers.addAll(Arrays.asList(rawHeaders.split(",")));
    }

    headers.add("Authorization=Bearer " + accessToken);

    customizedCfg.put("otel.exporter.otlp.headers", String.join(",", headers));

    String dash0Endpoint = originalCfg.getString(DASH0_EXTENSION_ENDPOINT);
    if (!Strings.isBlank(dash0Endpoint)) {
      // This truncation is needed because the Dash0 operator is currently adding the suffix
      // "/v1/traces" to the endpoint, and we don't want to duplicate it.
      dash0Endpoint = stripTracesSuffix(dash0Endpoint);
      setIfNotSet(originalCfg, customizedCfg, "otel.exporter.otlp.endpoint", dash0Endpoint);
    } else {
      /*
       * Upsert only if not set by the user, this allows the user to override the endpoint
       */
      setIfNotSet(originalCfg, customizedCfg, "otel.exporter.otlp.endpoint", DASH0_EXTENSION_ENDPOINT_URL);
    }
    setIfNotSet(originalCfg, customizedCfg, "otel.exporter.otlp.protocol", "http/protobuf");

    /*
     * Disable the metrics exporter
     */
    setIfNotSet(originalCfg, customizedCfg, "otel.metrics.exporter", "none");

    /*
     * Set limits in terms of span attribute length to match those that we have
     * in the ingestion pipeline.
     */
    setIfNotSet(originalCfg, customizedCfg, "otel.span.attribute.value.length.limit", "1024");

    /*
     * Configure span batching.
     */
    setIfNotSet(originalCfg, customizedCfg, "otel.bsp.schedule.delay", "10ms");
    setIfNotSet(originalCfg, customizedCfg, "otel.bsp.max.export.batch.size", "100");
    setIfNotSet(originalCfg, customizedCfg, "otel.bsp.export.timeout", "1s");

    return customizedCfg;
  }

  private static void setIfNotSet(
      ConfigProperties originalCfg, Map<String, String> customizedCfg, String key, String value) {

    if (Strings.isBlank(originalCfg.getString(key))) {
      customizedCfg.put(key, value);
    }
  }

  private Map<String, String> getDefaultProperties() {
    return Collections.singletonMap("otel.traces.sampler", "always_on");
  }

  static String stripTracesSuffix(String endpoint) {
    int suffixIndex = endpoint.indexOf("/v1/traces");
    if (suffixIndex > 0) {
      return endpoint.substring(0, suffixIndex);
    }
    return endpoint;
  }
}
