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
package io.dash0.javaagent.instrumentation.netty.v4_1;

import static io.dash0.javaagent.masking.MaskingCanary.FIELD;
import static io.dash0.javaagent.masking.MaskingCanary.HEADER;
import static io.dash0.javaagent.masking.MaskingCanary.SECRET;
import static org.awaitility.Awaitility.await;

import io.dash0.javaagent.masking.MaskingCanary;
import io.opentelemetry.instrumentation.test.utils.PortUtils;
import io.opentelemetry.instrumentation.testing.junit.AgentInstrumentationExtension;
import java.util.concurrent.TimeUnit;
import org.junit.jupiter.api.AfterAll;
import org.junit.jupiter.api.BeforeAll;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.extension.RegisterExtension;

/** Fails when this instrumentation captures a payload that MaskingSpanExporter doesn't mask. */
public class MaskingCanaryTest {
  @RegisterExtension
  static final AgentInstrumentationExtension instrumentation =
      AgentInstrumentationExtension.create();

  private static NettyServer server;
  private static int serverPort;

  @BeforeAll
  public static void setup() throws InterruptedException {
    serverPort = PortUtils.findOpenPort();
    server = new NettyServer(serverPort);
    server.start();
    await().atMost(5, TimeUnit.SECONDS).until(() -> server.isReady());
  }

  @AfterAll
  public static void teardown() {
    server.stop();
  }

  @Test
  void masksEverythingTheInstrumentationCaptures() throws InterruptedException {
    String body = "{\"" + FIELD + "\":\"" + SECRET + "\",\"orderId\":\"42\"}";
    new NettyHttpClient("localhost", serverPort)
        .sendPostRequest("/canary?" + FIELD + "=" + SECRET, body, HEADER, SECRET);

    // the client and the server span
    MaskingCanary.assertMaskedByKey(instrumentation.waitForTraces(1).get(0));
  }
}
