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
import java.util.Map;
import java.util.concurrent.ConcurrentHashMap;
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

  private static final Pattern BACKREFERENCE = Pattern.compile("\\\\(?:[1-9]|k<)");
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

  /** Keys longer than this are matched every time, so the cache can't grow without bound. */
  static final int MAX_CACHED_KEY_LENGTH = 128;

  static final int MAX_CACHED_KEYS = 4096;

  private final boolean maskAll;
  private final List<Pattern> patterns;

  /**
   * The patterns joined into one, so that a key is checked with one matcher instead of one per
   * pattern. Some patterns can't be joined and are kept in {@link #separate}: backreferences,
   * because joining renumbers the groups, and patterns that swallow the closing parenthesis, such
   * as an unterminated {@code \\Q} or a {@code (?x)} comment.
   */
  private final Pattern combined;

  private final List<Pattern> separate;

  /** Whether each key matches. Payloads use the same key names over and over. */
  private final Map<String, Boolean> cache = new ConcurrentHashMap<>();

  private MaskingRules(boolean maskAll, List<Pattern> patterns) {
    this.maskAll = maskAll;
    this.patterns = patterns;
    List<String> joinable = new ArrayList<>();
    List<Pattern> separate = new ArrayList<>();
    for (Pattern pattern : patterns) {
      String group = "(?:" + stripWildcards(pattern.pattern()) + ")";
      if (BACKREFERENCE.matcher(pattern.pattern()).find() || !compiles(group)) {
        separate.add(pattern);
      } else {
        joinable.add(group);
      }
    }
    this.combined =
        joinable.isEmpty()
            ? null
            : Pattern.compile(String.join("|", joinable), Pattern.CASE_INSENSITIVE);
    this.separate = separate;
  }

  /**
   * Drops a leading and a trailing {@code .*}. With {@link java.util.regex.Matcher#find()} they
   * don't change whether a key matches, but they make the regex engine backtrack over the whole
   * key. Patterns where the {@code .*} might mean something else are kept as they are.
   */
  static String stripWildcards(String pattern) {
    if (pattern.contains("\\Q")) {
      return pattern;
    }
    String stripped = pattern;
    if (stripped.startsWith(".*")
        && (stripped.length() == 2 || "?+*{".indexOf(stripped.charAt(2)) < 0)) {
      stripped = stripped.substring(2);
    }
    if (stripped.endsWith(".*") && !isEscaped(stripped, stripped.length() - 2)) {
      stripped = stripped.substring(0, stripped.length() - 2);
    }
    return stripped;
  }

  private static boolean compiles(String regex) {
    try {
      Pattern.compile(regex, Pattern.CASE_INSENSITIVE);
      return true;
    } catch (PatternSyntaxException e) {
      return false;
    }
  }

  /** True when the character at index is preceded by an odd number of backslashes. */
  private static boolean isEscaped(String pattern, int index) {
    int backslashes = 0;
    for (int i = index - 1; i >= 0 && pattern.charAt(i) == '\\'; i--) {
      backslashes++;
    }
    return backslashes % 2 == 1;
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
    if (key.length() > MAX_CACHED_KEY_LENGTH) {
      return find(key);
    }
    Boolean cached = cache.get(key);
    if (cached != null) {
      return cached;
    }
    boolean matches = find(key);
    if (cache.size() < MAX_CACHED_KEYS) {
      cache.put(key, matches);
    }
    return matches;
  }

  private boolean find(String key) {
    if (combined != null && combined.matcher(key).find()) {
      return true;
    }
    for (Pattern pattern : separate) {
      if (pattern.matcher(key).find()) {
        return true;
      }
    }
    return false;
  }

  /** Checks each pattern on its own, as the extension does. Tests compare it with matches(). */
  boolean matchesEachPattern(String key) {
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
