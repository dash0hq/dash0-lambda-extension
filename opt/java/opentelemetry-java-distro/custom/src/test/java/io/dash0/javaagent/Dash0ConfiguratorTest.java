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

import org.junit.jupiter.api.Test;

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
}
