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
package io.dash0.javaagent.instrumentation.kafkaclients.v0_11;

import com.google.auto.service.AutoService;
import io.opentelemetry.javaagent.extension.instrumentation.InstrumentationModule;
import io.opentelemetry.javaagent.extension.instrumentation.TypeInstrumentation;
import java.util.Collections;
import java.util.List;

@AutoService(InstrumentationModule.class)
public class KafkaClientInstrumentationModule extends InstrumentationModule {
  public KafkaClientInstrumentationModule() {
    super(
        "dash0-kafka-clients-producer-payloads",
        "dash0-kafka-clients-0.11",
        "dash0-kafka",
        "dash0-kafka-producer-payload");
  }

  @Override
  public List<TypeInstrumentation> typeInstrumentations() {
    return Collections.singletonList(new KafkaProducerPayloadInstrumentation());
  }

  @Override
  public boolean isHelperClass(String className) {
    return className.startsWith("io.dash0.javaagent.instrumentation.kafkaclients.v0_11.");
  }

  @Override
  public int order() {
    // Run after OTeL kafka Instrumentation
    return 1;
  }
}
