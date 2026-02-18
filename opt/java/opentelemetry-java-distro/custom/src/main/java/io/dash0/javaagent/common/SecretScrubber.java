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
package io.dash0.javaagent.common;

import java.util.Arrays;
import java.util.List;

public abstract class SecretScrubber extends AbstractRegExParser {
  public static final int DEFAULT_ATTRIBUTE_VALUE_LENGTH_LIMIT = 2048;
  public static final String LUMIGO_SECRET_MASKING_REGEX = "lumigo.secret.masking.regex";
  public static final String LUMIGO_SECRET_MASKING_ALL_MAGIC = "all";
  public static final String SCRUBBED_VALUE = "****";

  public static final List<String> DEFAULT_REGEX_KEYS =
      Arrays.asList(
          ".*pass.*",
          ".*key.*",
          ".*secret.*",
          ".*credential.*",
          ".*passphrase.*",
          ".*token.*",
          "SessionToken",
          "x-amz-security-token",
          "Signature",
          "Credential",
          "Authorization");

  @Override
  protected String getEnvVarName() {
    return LUMIGO_SECRET_MASKING_REGEX;
  }

  @Override
  protected List<String> getDefaultRegexKeys() {
    return DEFAULT_REGEX_KEYS;
  }

  @Override
  protected String getMagicValue() {
    return LUMIGO_SECRET_MASKING_ALL_MAGIC;
  }
}
