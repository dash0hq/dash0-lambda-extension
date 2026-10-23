from dash0_opentelemetry.libs.masking import (
    DEFAULT_MASKING_PATTERNS,
    get_rule,
    mask_and_dump,
    parse_query_string,
)


def test_parse_query_string_keeps_repeated_and_blank_parameters():
    assert parse_query_string(b"a=1&b=&a=2&a=3&c=%40") == {
        "a": ["1", "2", "3"],
        "b": "",
        "c": "@",
    }


def test_defaults_apply_without_any_variable(monkeypatch):
    for name in ("LUMIGO_SECRET_MASKING_REGEX", "LUMIGO_BLACKLIST_REGEX"):
        monkeypatch.delenv(name, raising=False)
    rule = get_rule("requestBody")
    assert rule.patterns == tuple(DEFAULT_MASKING_PATTERNS)
    assert rule.matches("X-Api-Token") and rule.matches("passphrase")
    assert not rule.matches("email")


def test_reads_the_variables_on_every_call(monkeypatch):
    monkeypatch.setenv("LUMIGO_SECRET_MASKING_REGEX", '["email"]')
    assert mask_and_dump({"x-email": "e"}, None, 100) == '{"x-email": "****"}'
    monkeypatch.setenv("LUMIGO_SECRET_MASKING_REGEX", "all")
    assert mask_and_dump({"x-email": "e"}, None, 100) == '"****"'


def test_masks_mappings_that_are_not_dicts():
    from types import MappingProxyType

    assert (
        mask_and_dump(MappingProxyType({"Authorization": "a"}), "requestHeaders", 100)
        == '{"Authorization": "****"}'
    )


def test_masks_inside_lists_and_json_arrays():
    assert (
        mask_and_dump('[{"password": "p"}, "x"]', "requestBody", 100)
        == '[{"password": "****"}, "x"]'
    )


def test_masks_before_truncating():
    dumped = mask_and_dump({"password": "p" * 50, "id": 1}, "requestBody", 20)
    assert "ppp" not in dumped and dumped.endswith("...[too long]")


def test_stops_masking_once_the_output_is_truncated_anyway():
    items = [{"password": "p", "id": i} for i in range(10000)]
    dumped = mask_and_dump(items, "requestBody", 100)
    assert dumped.endswith("...[too long]")
    assert 80 <= len(dumped) - len("...[too long]") <= 100
    assert '"password": "p"' not in dumped


def test_falls_back_to_one_regex_per_pattern_for_inline_flags(monkeypatch):
    monkeypatch.setenv("LUMIGO_SECRET_MASKING_REGEX", '["(?i)email", "pass"]')
    rule = get_rule()
    assert rule.matches("X-EMAIL") and rule.matches("password")
    assert not rule.matches("orderId")


def test_outer_wildcards_are_dropped_only_where_search_finds_the_same_keys():
    from dash0_opentelemetry.libs.masking import _without_outer_wildcards as strip

    assert strip(".*pass.*") == "pass"
    assert strip(".*a|b.*") == "a|b"
    assert strip(".*") == ""
    assert strip("^email$") == "^email$"
    assert strip(".*?x") == ".*?x"
    assert strip(r"a\.*") == r"a\.*"
    assert strip(r"a\\.*") == "a\\\\"
