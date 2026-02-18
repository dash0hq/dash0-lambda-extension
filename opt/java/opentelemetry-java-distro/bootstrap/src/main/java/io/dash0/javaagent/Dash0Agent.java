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

import io.opentelemetry.javaagent.OpenTelemetryAgent;
import java.lang.instrument.Instrumentation;

public class Dash0Agent {
  public static void premain(final String agentArgs, final Instrumentation inst) {
    agentmain(agentArgs, inst);
  }

  public static void agentmain(final String agentArgs, final Instrumentation inst) {
    if (isDebugMode()) {
      System.setProperty("otel.javaagent.debug", "true");
      System.setProperty("io.opentelemetry.javaagent.slf4j.simpleLogger.defaultLogLevel", "debug");
      System.setProperty("otel.log.level", "debug");
      System.setProperty("dash0.debug", "true");
    }
    if (is_switch_off()) {
      System.err.println(
          "Dash0 OpenTelemetry Java distribution disabled via the 'DASH0_SWITCH_OFF' environment variable");
      return;
    }
    System.out.println(
        "Loading the Dash0 OpenTelemetry Java distribution (version "
            + Dash0Version.VERSION
            + ")");
    OpenTelemetryAgent.agentmain(agentArgs, inst);
  }

  private static boolean isDebugMode() {
    String value = System.getProperty("otel.javaagent.debug");
    if (value == null) {
      value = System.getenv("OTEL_JAVAAGENT_DEBUG");
    }
    if (value == null) {
      value = System.getProperty("dash0.debug");
    }
    if (value == null) {
      value = System.getenv("DASH0_DISTRO_DEBUG");
    }
    return Boolean.parseBoolean(value);
  }

  private static boolean is_switch_off() {
    String value = System.getProperty("dash0.switch_off");
    if (value == null) {
      value = System.getenv("DASH0_SWITCH_OFF");
    }
    return Boolean.parseBoolean(value);
  }
}
