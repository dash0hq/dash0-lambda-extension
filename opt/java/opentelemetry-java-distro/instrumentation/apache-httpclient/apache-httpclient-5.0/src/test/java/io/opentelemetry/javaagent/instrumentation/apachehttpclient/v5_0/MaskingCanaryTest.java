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
package io.opentelemetry.javaagent.instrumentation.apachehttpclient.v5_0;

import static com.github.tomakehurst.wiremock.client.WireMock.aResponse;
import static com.github.tomakehurst.wiremock.client.WireMock.post;
import static com.github.tomakehurst.wiremock.client.WireMock.urlPathEqualTo;
import static com.github.tomakehurst.wiremock.core.WireMockConfiguration.wireMockConfig;
import static io.dash0.javaagent.masking.MaskingCanary.FIELD;
import static io.dash0.javaagent.masking.MaskingCanary.HEADER;
import static io.dash0.javaagent.masking.MaskingCanary.SECRET;

import com.github.tomakehurst.wiremock.core.Options;
import com.github.tomakehurst.wiremock.junit5.WireMockExtension;
import io.dash0.javaagent.masking.MaskingCanary;
import io.opentelemetry.instrumentation.testing.junit.AgentInstrumentationExtension;
import org.apache.hc.client5.http.impl.classic.CloseableHttpClient;
import org.apache.hc.client5.http.impl.classic.HttpClients;
import org.apache.hc.core5.http.ContentType;
import org.apache.hc.core5.http.io.entity.EntityUtils;
import org.apache.hc.core5.http.io.entity.StringEntity;
import org.apache.hc.core5.http.io.support.ClassicRequestBuilder;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.extension.RegisterExtension;

/** Fails when this instrumentation captures a payload that MaskingSpanExporter doesn't mask. */
public class MaskingCanaryTest {
  @RegisterExtension
  static final AgentInstrumentationExtension instrumentation = AgentInstrumentationExtension.create();

  @RegisterExtension
  static final WireMockExtension mockServer = WireMockExtension.newInstance()
      .options(wireMockConfig().dynamicPort()
          .gzipDisabled(true)
          .useChunkedTransferEncoding(Options.ChunkedEncodingPolicy.NEVER))
      .build();

  @Test
  void masksEverythingTheInstrumentationCaptures() throws Exception {
    String body = "{\"" + FIELD + "\":\"" + SECRET + "\",\"orderId\":\"42\"}";
    mockServer.stubFor(post(urlPathEqualTo("/canary"))
        .willReturn(aResponse()
            .withHeader("Content-Type", "application/json")
            .withHeader(HEADER, SECRET)
            .withBody(body)));

    try (CloseableHttpClient client = HttpClients.createDefault()) {
      client.execute(
          ClassicRequestBuilder.post(mockServer.url("/canary?" + FIELD + "=" + SECRET))
              .setEntity(new StringEntity(body, ContentType.APPLICATION_JSON))
              .addHeader(HEADER, SECRET)
              .build(),
          response -> EntityUtils.toString(response.getEntity()));
    }

    MaskingCanary.assertMaskedByKey(instrumentation.waitForTraces(1).get(0));
  }
}
