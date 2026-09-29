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
package io.dash0.javaagent.instrumentation.grpc.v1_6;

import static io.opentelemetry.javaagent.extension.matcher.AgentElementMatchers.hasClassesNamed;
import static net.bytebuddy.matcher.ElementMatchers.named;
import static net.bytebuddy.matcher.ElementMatchers.takesArguments;

import io.grpc.ClientCall;
import io.opentelemetry.api.trace.Span;
import io.opentelemetry.instrumentation.api.util.VirtualField;
import io.opentelemetry.javaagent.extension.instrumentation.TypeInstrumentation;
import io.opentelemetry.javaagent.extension.instrumentation.TypeTransformer;
import net.bytebuddy.asm.Advice;
import net.bytebuddy.description.type.TypeDescription;
import net.bytebuddy.matcher.ElementMatcher;

/**
 * Hands the client span that {@link ClientCallInstrumentation} stored on a forwarding listener (the
 * upstream tracing listener) down to the listener it forwards to (the application's listener).
 */
public class ForwardingClientCallListenerInstrumentation implements TypeInstrumentation {
  @Override
  public ElementMatcher<ClassLoader> classLoaderOptimization() {
    return hasClassesNamed("io.grpc.ForwardingClientCallListener");
  }

  @Override
  public ElementMatcher<TypeDescription> typeMatcher() {
    return named("io.grpc.ForwardingClientCallListener$SimpleForwardingClientCallListener");
  }

  @Override
  public void transform(TypeTransformer transformer) {
    transformer.applyAdviceToMethod(
        named("delegate").and(takesArguments(0)),
        ForwardingClientCallListenerInstrumentation.class.getName() + "$DelegateAdvice");
  }

  @SuppressWarnings("unused")
  public static class DelegateAdvice {
    @Advice.OnMethodExit(suppress = Throwable.class)
    public static void methodExit(
        @Advice.This ClientCall.Listener<?> listener,
        @Advice.Return ClientCall.Listener<?> delegate) {
      if (delegate == null) {
        return;
      }
      VirtualField<ClientCall.Listener<?>, Span> virtualField =
          VirtualField.find(ClientCall.Listener.class, Span.class);
      Span span = virtualField.get(listener);
      if (span != null && virtualField.get(delegate) == null) {
        virtualField.set(delegate, span);
      }
    }
  }
}
