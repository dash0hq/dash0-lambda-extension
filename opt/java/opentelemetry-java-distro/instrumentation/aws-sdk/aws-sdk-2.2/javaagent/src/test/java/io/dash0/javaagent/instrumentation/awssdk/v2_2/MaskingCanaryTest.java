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
package io.dash0.javaagent.instrumentation.awssdk.v2_2;

import static io.dash0.javaagent.masking.MaskingCanary.FIELD;
import static io.dash0.javaagent.masking.MaskingCanary.SECRET;

import io.dash0.javaagent.masking.MaskingCanary;
import io.opentelemetry.instrumentation.test.utils.PortUtils;
import io.opentelemetry.instrumentation.testing.junit.AgentInstrumentationExtension;
import io.opentelemetry.sdk.trace.data.SpanData;
import java.net.URI;
import java.util.List;
import java.util.stream.Collectors;
import org.elasticmq.rest.sqs.SQSRestServer;
import org.elasticmq.rest.sqs.SQSRestServerBuilder;
import org.junit.jupiter.api.AfterAll;
import org.junit.jupiter.api.BeforeAll;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.extension.RegisterExtension;
import software.amazon.awssdk.auth.credentials.AwsBasicCredentials;
import software.amazon.awssdk.auth.credentials.StaticCredentialsProvider;
import software.amazon.awssdk.regions.Region;
import software.amazon.awssdk.services.sqs.SqsClient;

/**
 * Fails when this instrumentation captures a payload that MaskingSpanExporter doesn't mask. This
 * SDK version sends SQS calls form-encoded and gets XML back, so only {@code all} masks them.
 */
public class MaskingCanaryTest {
  @RegisterExtension
  static final AgentInstrumentationExtension instrumentation =
      AgentInstrumentationExtension.create();

  private static SQSRestServer sqsServer;
  private static SqsClient client;
  private static int sqsServerPort;

  @BeforeAll
  static void setUp() throws Exception {
    sqsServerPort = PortUtils.findOpenPort();
    sqsServer = SQSRestServerBuilder.withPort(sqsServerPort).withInterface("localhost").start();
    client =
        SqsClient.builder()
            .endpointOverride(new URI("http://localhost:" + sqsServerPort))
            .region(Region.US_EAST_1)
            .credentialsProvider(
                StaticCredentialsProvider.create(AwsBasicCredentials.create("x", "x")))
            .build();
  }

  @AfterAll
  static void tearDown() {
    sqsServer.stopAndWait();
  }

  @Test
  void masksEverythingTheInstrumentationCaptures() {
    client.createQueue(builder -> builder.queueName("canary"));
    String queueUrl = "http://localhost:" + sqsServerPort + "/000000000000/canary";
    client.sendMessage(
        builder ->
            builder.queueUrl(queueUrl).messageBody("{\"" + FIELD + "\":\"" + SECRET + "\"}"));
    client.receiveMessage(builder -> builder.queueUrl(queueUrl));

    // CreateQueue, SendMessage with the processing of the received message, ReceiveMessage
    List<SpanData> spans =
        instrumentation.waitForTraces(3).stream()
            .flatMap(List::stream)
            .collect(Collectors.toList());
    MaskingCanary.assertMaskedWithAll(spans);
  }
}
