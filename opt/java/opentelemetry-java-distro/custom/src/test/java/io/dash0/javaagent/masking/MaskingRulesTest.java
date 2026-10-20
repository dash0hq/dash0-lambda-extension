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
package io.dash0.javaagent.masking;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertNotNull;
import static org.junit.jupiter.api.Assertions.assertNull;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.ArrayList;
import java.util.List;
import java.util.regex.Matcher;
import java.util.regex.Pattern;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.params.ParameterizedTest;
import org.junit.jupiter.params.provider.ValueSource;

class MaskingRulesTest {
  /** The repository root, found from the module directory Gradle runs the tests in. */
  private static Path repoRoot() {
    Path dir = Paths.get("").toAbsolutePath();
    while (dir != null && !Files.exists(dir.resolve("src/otlp/masking.rs"))) {
      dir = dir.getParent();
    }
    assertNotNull(dir, "src/otlp/masking.rs not found above " + Paths.get("").toAbsolutePath());
    return dir;
  }

  private static List<String> quotedStrings(String text) {
    List<String> strings = new ArrayList<>();
    Matcher matcher = Pattern.compile("\"([^\"]*)\"").matcher(text);
    while (matcher.find()) {
      strings.add(matcher.group(1));
    }
    return strings;
  }

  @Test
  void defaultsMatchTheExtension() throws IOException {
    String rust =
        new String(
            Files.readAllBytes(repoRoot().resolve("src/otlp/masking.rs")), StandardCharsets.UTF_8);
    Matcher matcher =
        Pattern.compile("DEFAULT_MASKING_PATTERNS: &\\[&str\\] = &\\[(.*?)\\];", Pattern.DOTALL)
            .matcher(rust);
    assertTrue(matcher.find());
    assertEquals(quotedStrings(matcher.group(1)), MaskingRules.DEFAULT_PATTERNS);
  }

  @Test
  void defaultsMatchTheWrapper() throws IOException {
    String shared =
        new String(Files.readAllBytes(repoRoot().resolve("opt/shared.sh")), StandardCharsets.UTF_8);
    Matcher matcher = Pattern.compile("DASH0_DEFAULT_MASK_RULES='(.*?)'").matcher(shared);
    assertTrue(matcher.find());
    assertEquals(quotedStrings(matcher.group(1)), MaskingRules.DEFAULT_PATTERNS);
  }

  @Test
  void patternsMatchAnywhereInTheKeyIgnoringCase() {
    MaskingRules rules = MaskingRules.parse("DASH0_MASK_RULES", "[\"email\"]");
    assertTrue(rules.matches("x-email"));
    assertTrue(rules.matches("EMAIL"));
    assertFalse(rules.matches("orderId"));
  }

  @Test
  void anchoredPatternsMatchOnlyTheExactKey() {
    MaskingRules rules = MaskingRules.parse("DASH0_MASK_RULES", "[\"^email$\"]");
    assertTrue(rules.matches("Email"));
    assertFalse(rules.matches("x-email"));
  }

  @Test
  void allMasksEverything() {
    MaskingRules rules = MaskingRules.parse("DASH0_MASK_RULES", "all");
    assertTrue(rules.masksAll());
    assertTrue(rules.matches("orderId"));
  }

  @Test
  void emptyListMasksNothing() {
    MaskingRules rules = MaskingRules.parse("DASH0_MASK_RULES", "[]");
    assertFalse(rules.masksAll());
    assertFalse(rules.matches("password"));
  }

  @Test
  void invalidRegexIsDropped() {
    MaskingRules rules = MaskingRules.parse("DASH0_MASK_RULES", "[\"(\", \".*email.*\"]");
    assertTrue(rules.matches("email"));
    assertFalse(rules.matches("("));
  }

  // The same values opt/test/shared.test.sh rejects in the wrapper.
  @ParameterizedTest
  @ValueSource(
      strings = {
        ".*email.*",
        "[.*email.*]",
        "[\".*email.*\",]",
        "[null]",
        "[\"a\", 1]",
        "{\"a\": \"b\"}",
        "\"all\"",
        "ALL",
        "[\"a\"] trailing",
        "[\"a\""
      })
  void invalidValuesAreIgnored(String value) {
    assertNull(MaskingRules.parse("DASH0_MASK_RULES", value));
  }

  @ParameterizedTest
  @ValueSource(strings = {"", " "})
  void blankValuesAreIgnored(String value) {
    assertNull(MaskingRules.parse("DASH0_MASK_RULES", value));
  }
}
