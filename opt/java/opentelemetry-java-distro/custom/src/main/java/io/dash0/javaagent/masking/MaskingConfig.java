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

import io.opentelemetry.sdk.autoconfigure.spi.ConfigProperties;

/**
 * The masking rules for each payload type, read from the DASH0_MASK_* environment variables (or the
 * matching dash0.mask.* system properties). A per-payload variable wins over DASH0_MASK_RULES,
 * which wins over the defaults. Unset or invalid values fall back to the next in line.
 */
public final class MaskingConfig {
  public static final String DASH0_MASK_RULES = "dash0.mask.rules";
  public static final String DASH0_MASK_REQUEST_BODY = "dash0.mask.request.body";
  public static final String DASH0_MASK_RESPONSE_BODY = "dash0.mask.response.body";
  public static final String DASH0_MASK_REQUEST_HEADERS = "dash0.mask.request.headers";
  public static final String DASH0_MASK_RESPONSE_HEADERS = "dash0.mask.response.headers";
  public static final String DASH0_MASK_QUERY_PARAMS = "dash0.mask.query.params";

  final MaskingRules requestBody;
  final MaskingRules responseBody;
  final MaskingRules requestHeaders;
  final MaskingRules responseHeaders;
  final MaskingRules queryParams;

  private MaskingConfig(
      MaskingRules requestBody,
      MaskingRules responseBody,
      MaskingRules requestHeaders,
      MaskingRules responseHeaders,
      MaskingRules queryParams) {
    this.requestBody = requestBody;
    this.responseBody = responseBody;
    this.requestHeaders = requestHeaders;
    this.responseHeaders = responseHeaders;
    this.queryParams = queryParams;
  }

  public static MaskingConfig fromConfig(ConfigProperties cfg) {
    MaskingRules rules = rulesOf(cfg, DASH0_MASK_RULES, MaskingRules.DEFAULTS);
    return new MaskingConfig(
        rulesOf(cfg, DASH0_MASK_REQUEST_BODY, rules),
        rulesOf(cfg, DASH0_MASK_RESPONSE_BODY, rules),
        rulesOf(cfg, DASH0_MASK_REQUEST_HEADERS, rules),
        rulesOf(cfg, DASH0_MASK_RESPONSE_HEADERS, rules),
        rulesOf(cfg, DASH0_MASK_QUERY_PARAMS, rules));
  }

  private static MaskingRules rulesOf(ConfigProperties cfg, String key, MaskingRules fallback) {
    MaskingRules rules = MaskingRules.parse(toEnvVarName(key), cfg.getString(key));
    return rules != null ? rules : fallback;
  }

  private static String toEnvVarName(String key) {
    return key.toUpperCase().replace('.', '_');
  }
}
