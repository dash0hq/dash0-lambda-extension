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

import com.fasterxml.jackson.core.JsonFactory;
import com.fasterxml.jackson.core.JsonParser;
import com.fasterxml.jackson.core.JsonToken;
import io.dash0.javaagent.utils.Strings;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.Collections;
import java.util.List;
import java.util.logging.Level;
import java.util.logging.Logger;
import java.util.regex.Pattern;
import java.util.regex.PatternSyntaxException;
import java.util.stream.Collectors;

/**
 * The masking rules of one payload type: either {@code all}, which masks the whole payload, or a
 * list of regular expressions matched against key names.
 */
public final class MaskingRules {
  private static final Logger LOGGER = Logger.getLogger(MaskingRules.class.getName());
  private static final JsonFactory JSON_FACTORY = new JsonFactory();

  public static final String MASKED_VALUE = "****";
  static final String ALL = "all";

  /**
   * Must match DEFAULT_MASKING_PATTERNS in src/otlp/masking.rs and DASH0_DEFAULT_MASK_RULES in
   * opt/shared.sh. MaskingRulesTest checks the three against each other.
   */
  public static final List<String> DEFAULT_PATTERNS =
      Collections.unmodifiableList(
          Arrays.asList(
              ".*pass.*",
              ".*key.*",
              ".*secret.*",
              ".*credential.*",
              ".*passphrase.*",
              ".*token.*",
              ".*signature.*",
              ".*authorization.*"));

  public static final MaskingRules DEFAULTS = ofPatterns(DEFAULT_PATTERNS);

  private final boolean maskAll;
  private final List<Pattern> patterns;

  private MaskingRules(boolean maskAll, List<Pattern> patterns) {
    this.maskAll = maskAll;
    this.patterns = patterns;
  }

  static MaskingRules all() {
    return new MaskingRules(true, Collections.emptyList());
  }

  /**
   * Compiles the patterns case-insensitively. A pattern that is not a valid regular expression is
   * dropped, as in the extension.
   */
  static MaskingRules ofPatterns(List<String> regexes) {
    List<Pattern> patterns = new ArrayList<>();
    for (String regex : regexes) {
      try {
        patterns.add(Pattern.compile(regex, Pattern.CASE_INSENSITIVE));
      } catch (PatternSyntaxException e) {
        LOGGER.log(Level.FINE, "Dropping invalid masking pattern " + regex, e);
      }
    }
    return new MaskingRules(false, Collections.unmodifiableList(patterns));
  }

  /**
   * Parses the value of a DASH0_MASK_* variable: {@code all} or a JSON array of strings. Returns
   * null when the value is unset or invalid, so that the caller falls back to the next rules in
   * line. The wrapper already warns about invalid values in the function log, so they are only
   * logged at FINE level here.
   */
  static MaskingRules parse(String name, String value) {
    if (Strings.isBlank(value)) {
      return null;
    }
    if (ALL.equals(value)) {
      return all();
    }
    List<String> regexes = new ArrayList<>();
    try (JsonParser parser = JSON_FACTORY.createParser(value)) {
      if (parser.nextToken() != JsonToken.START_ARRAY) {
        throw new IllegalArgumentException("not a JSON array");
      }
      JsonToken token;
      while ((token = parser.nextToken()) != JsonToken.END_ARRAY) {
        if (token != JsonToken.VALUE_STRING) {
          throw new IllegalArgumentException("not a string: " + token);
        }
        regexes.add(parser.getText());
      }
      if (parser.nextToken() != null) {
        throw new IllegalArgumentException("trailing content");
      }
    } catch (Exception e) {
      LOGGER.log(
          Level.FINE,
          "Ignoring " + name + ": expected a JSON array of regular expressions or \"all\"",
          e);
      return null;
    }
    return ofPatterns(regexes);
  }

  public boolean masksAll() {
    return maskAll;
  }

  /** True when a pattern matches anywhere in the key, ignoring case. */
  public boolean matches(String key) {
    if (maskAll) {
      return true;
    }
    for (Pattern pattern : patterns) {
      if (pattern.matcher(key).find()) {
        return true;
      }
    }
    return false;
  }

  @Override
  public String toString() {
    return maskAll
        ? ALL
        : patterns.stream()
            .map(Pattern::pattern)
            .collect(Collectors.joining("\",\"", "[\"", "\"]"));
  }
}
