"""Render measured results to the one authorized report (local staging first)."""
import json
from pathlib import Path

HERE = Path(__file__).resolve().parent
d = json.loads((HERE/'results.json').read_text())
r = d['results']
lines = ['# Python distro masking benchmark', '',
    'Measured 2026-10-23 on Apple M4 Pro, 14 CPU cores, 48 GiB RAM, macOS 26.6.2 (25G83), arm64. '
    'Existing venv: CPython 3.13.11. Other workers were active on this laptop; no CPU pinning, governor control, or isolation. '
    'These are local latency samples, not Lambda measurements.', '',
    f"Source commit: `{d['metadata']['git_head']}`. Packages: " + ', '.join(f'`{k}=={v}`' for k,v in d['metadata']['packages'].items()) + '.', '',
    '## Reproduce', '',
    'From `/Users/rm/code/dash0/dash0-claude/sig-700-python-masking`:', '', '```sh',
    'PYTHONDONTWRITEBYTECODE=1 opt/python/distro/.venv/bin/python opt/python/bench-scratch/bench.py',
    'opt/python/distro/.venv/bin/python opt/python/bench-scratch/report.py', '```', '',
    'Scripts, result summaries, cold samples, and captured outputs: `opt/python/bench-scratch/{bench.py,cold.py,report.py,results.json}`. '
    'The renderer stages this report at `opt/python/bench-scratch/python.md`; delivery copies that file to the requested design directory. '
    'No packages were installed and no tracked source files were changed.', '',
    'Exact shared fixture: `/Users/rm/code/dash0/dash0-claude/central-masking-extension/benches/fixtures/masking-payloads.json`. '
    f"SHA-256: `{d['metadata']['fixture_sha256']}`. Input lengths in UTF-8 bytes: " + ', '.join(f'{k}={v}' for k,v in d['metadata']['fixture_bytes'].items()) + '.', '',
    '## Method and scope', '',
    'Each timed operation has 1,500 warm-up calls, then 20,000 individually timed calls (spans: 10,000; 20-span invocations: 2,000). '
    'Timing uses `perf_counter_ns`; GC is disabled only during timing. p50 is the median and p99 is nearest-rank of individual calls, '
    'not a percentile of batch averages. Calls include the timer/function-call overhead (no subtraction). '
    'The large JSON is parsed, recursively copied/masked, serialized with the real `lumigo_core.lumigo_utils.aws_dump`, and subject to the real 4,096-character budget. '
    'No payloads are invented or padded.', '',
    'Default mode sets the fixture default pattern array in `DASH0_MASK_RULES` and `LUMIGO_SECRET_MASKING_REGEX`, equivalent to the wrapper mapping; '
    'custom mode uses exactly `[".*email.*"]` through the same variables. Custom rules **replace** the defaults. Context overrides and legacy rules are cleared. '
    'The measured environment had both OpenTelemetry attribute length variables unset (the 4,096-character default); the reproduction script pins them to 4,096. '
    'Shipped mode re-reads context and global environment variables on every call, with `_parse_rule` already warm. '
    'Cached mode changes only the in-memory benchmark module\'s `get_rule` to return a precomputed rule for each context; '
    'it is an experiment, not a source change or proposed implementation.', '',
    'Allocation columns are median **peak additional live traced bytes** and net live traced bytes retained after the call, '
    'over 101 separately measured calls, with `tracemalloc` and collection before each call. '
    'They include Python objects/temporaries visible to tracemalloc, exclude untraced native allocations, and are **not cumulative bytes allocated or allocation-event counts**. '
    'Retained values include the returned value, CPython object freelists, and small measurement effects; they are not solely output size. '
    'Peak is the useful working-memory estimate. Timing runs do not enable tracemalloc.', '',
    '## Per payload: mask_text (HTTP body text path)', '',
    '| Rules | Payload | Shipped p50 ns | Shipped p99 ns | Peak B | Retained B | Cached p50 ns | Cached p99 ns | Cached peak B |',
    '|---|---|---:|---:|---:|---:|---:|---:|---:|']

def fmt(x):
    return f'{x:,.0f}'

for mode in r:
    for name in d['metadata']['fixture_bytes']:
        a,b = (r[mode][variant]['mask_text/'+name] for variant in ['shipped','cached_rule'])
        lines.append('| '+ ' | '.join([mode,name,*[fmt(a[k]) for k in ['p50_ns','p99_ns','peak_traced_B','retained_B']],*[fmt(b[k]) for k in ['p50_ns','p99_ns','peak_traced_B']]]) + ' |')
lines += ['', '## Per payload: mask_and_dump (JSON serialization capture path)', '',
    'Same exact text fixtures passed directly to the real function. This path parses valid object/array JSON; '
    'other strings are JSON-quoted/escaped before truncation. It is used by `json_utils.dump_with_context` and by structured-body capture.', '',
    '| Rules | Payload | Shipped p50 ns | Shipped p99 ns | Peak B | Retained B | Cached p50 ns | Cached p99 ns | Cached peak B |',
    '|---|---|---:|---:|---:|---:|---:|---:|---:|']
for mode in r:
    for name in d['metadata']['fixture_bytes']:
        a,b = (r[mode][variant]['mask_and_dump/'+name] for variant in ['shipped','cached_rule'])
        lines.append('| '+ ' | '.join([mode,name,*[fmt(a[k]) for k in ['p50_ns','p99_ns','peak_traced_B','retained_B']],*[fmt(b[k]) for k in ['p50_ns','p99_ns','peak_traced_B']]]) + ' |')
lines += ['', '## Per span and invocation', '',
    'The exact `span_attributes.small_json` and `.json_4k` fixture sets are retained on a minimal recording span. '
    'Headers are decoded to mappings once outside timing because Requests already receives header mappings. '
    'Each measurement calls real `dump_with_context` for request/response headers and real `add_body_attribute` '
    'for request/response body text, then retains all outputs. Other fixture attributes (including `url.full`) are copied unchanged, '
    'matching the scope of the Python HTTP client hooks. Body-helper safe-execution and slicing costs and header limit lookup are included. '
    'It excludes network calls, Requests machinery, decoding `response.content` bytes, SDK span construction/locking, '
    'the enclosing request/response hook safe-execution contexts, and export/protobuf serialization. '
    'Thus this measures capture/masking cost for the supplied attribute set, not end-to-end tracing. '
    'There is no distro-wide export masking pass. The json_4k span fixture has one 4,111-byte request body and one small response body, exactly as supplied. '
    'Invocations execute and retain 20 full captures, rather than multiplying a percentile by 20.', '',
    '| Rules | Lookup | Fixture | Scope | p50 ns | p99 ns | Peak B | Retained B |',
    '|---|---|---|---|---:|---:|---:|---:|']
for mode in r:
    for variant in ['shipped','cached_rule']:
        for name in ['small_json','json_4k']:
            for scope in ['span','invocation20']:
                a = r[mode][variant][scope+'/'+name]
                lines.append('| '+' | '.join([mode,variant,name,scope,*[fmt(a[k]) for k in ['p50_ns','p99_ns','peak_traced_B','retained_B']]])+' |')
lines += ['', '## Environment lookup and server query parsing', '',
    'The query is extracted verbatim from fixture `url.full`: `api_key=k-1&page=2`. '
    '`query_parse_and_mask` uses the actual server path: `parse_query_string` followed by '
    '`dump_with_context("queryParams", ...)`, including its size-limit environment lookup. '
    'It emits a JSON object attribute (`http.request.query`); it does not rewrite the URL. '
    'URL extraction itself is outside timing because server hooks already receive a query string.', '',
    '| Rules | Lookup | Operation | p50 ns | p99 ns | Peak B |',
    '|---|---|---|---:|---:|---:|']
for mode in r:
    for variant in ['shipped','cached_rule']:
        for name in ['get_rule_only','query_parse_only','query_parse_and_mask']:
            a = r[mode][variant][name]
            lines.append('| '+' | '.join([mode,variant,name,*[fmt(a[k]) for k in ['p50_ns','p99_ns','peak_traced_B']]])+' |')
for mode in r:
    a,b = (r[mode][v]['get_rule_only']['p50_ns'] for v in ['shipped','cached_rule'])
    lines += ['',f'Direct `{mode}` rule-lookup median delta: **{fmt(a-b)} ns/call** ({fmt(a)} ns shipped vs {fmt(b)} ns cached). '
              'This is the isolated environment lookup/cache-key overhead; payload-level differences also contain scheduling and timing variation.']
lines += ['', '## Cold module import and first call', '',
    '30 fresh CPython processes per rules/payload pair. Only `os`, `sys`, and `time` are explicitly imported before the timer. '
    'The import interval includes importing the package and masking dependencies, class/function loading, default regex compilation, '
    'and then the first `mask_text` interval includes the first global env-value parsing/regex compilation plus actual masking. '
    'Python interpreter/site startup, argument/fixture setup, and process launch are outside the measured interval. '
    'Filesystem caches are warm and existing bytecode may be used; these are fresh-process imports, not cold-disk I/O. '
    '`DASH0_SWITCH_OFF=true` prevents full tracer/instrumentation activation and `DASH0_DISABLE_PYTHON_DEPENDENCY_CHECK=true` '
    'skips dependency scanning, so this isolates import/masking startup and is not full distro cold-start cost.', '',
    '| Rules | Fixture | Interval | p50 ns | p99 ns |', '|---|---|---|---:|---:|']
for mode, cases in d['cold'].items():
    for name, measurements in cases.items():
        for interval in ['import_ns','first_call_ns','total_ns']:
            a = measurements[interval]
            lines.append('| '+' | '.join([mode,name,interval,fmt(a['p50_ns']),fmt(a['p99_ns'])])+' |')
lines += ['', '## Correctness and truncation', '',
    '| Fixture | Default rules | Custom email rule |', '|---|---|---|',
    '| small_json | password/apiToken become `****`; email remains visible | email becomes `****`; password/apiToken remain visible by configured replacement semantics |',
    '| json_4k | password/apiToken masked before budget truncation | email masked before budget truncation; password/apiToken visible |',
    '| truncated_json_4k | Invalid/incomplete JSON: mask_text returns original text, including password/apiToken (**gap**) | Returns original text, including email (**gap**) |',
    '| form_encoded | No form parser on body path: password/csrf_token remain visible (**gap**) | Unchanged; fixture has no email parameter |',
    '| non_matching_json_4k | No default-matching keys; content preserved subject to reserialization/budget truncation | Contains an email key despite fixture name: email correctly becomes `****`, subject to reserialization/budget truncation |',
    '| plain_text | Opaque text returned unchanged, correctly no keyed values | Same |',
    '| URL query through server query parser | JSON query masks api_key; page stays 2 | api_key stays visible because email-only replaces defaults |',
    '| url.full in client span | Untouched by Dash0 masking; api_key remains visible (**gap**) | Untouched; no email key in fixture URL |', '',
    'For non-JSON strings, `mask_and_dump` quotes/escapes the string but does not hide embedded keyed secrets. '
    'For the supplied span: default rules mask request header authorization and response body sessionToken; '
    'custom email rules retain both and mask only email in the request body. `set-cookie` is not matched by either supplied rule set. '
    'The installed upstream Requests instrumentation calls `redact_url`, whose fixed list is '
    '`AWSAccessKeyId`, `Signature`, `sig`, and `X-Goog-Signature`; it does not mask the supplied `api_key`. '
    'Known fixture gaps agree with `src/test/unit/masking/test_known_masking_gaps.py` (client URL/query and non-JSON body cases).', '',
    'Masking happens **before** attribute length truncation. `_parse_json_text` parses the original complete string, '
    '`_mask` masks while building an output with an approximate character budget, and `_dump_masked` serializes then '
    'slices to max_size and appends `...[too long]` if needed. `add_body_attribute` slices the result again to 4,096 characters; '
    'the SDK also has its configured attribute limit. Thus the output can be incomplete JSON even though visible secrets '
    'were masked first. If the input is already truncated JSON, parsing fails and no key masking occurs. '
    'All raw function and span outputs for both rule sets are saved in `results.json` for inspection.', '',
    'Source paths: `opt/python/distro/src/dash0_opentelemetry/libs/{masking.py,json_utils.py,general_utils.py}`, '
    '`instrumentations/instrumentation_utils.py`, `instrumentations/requests/__init__.py`, '
    '`instrumentations/{fastapi/parsers,flask,django}/__init__.py`, and `opt/shared.sh:map_masking_env`.', '',
    '## Comparison with supplied Rust extension measurements', '',
    'Provided Criterion release numbers (not remeasured here): small_json 198 ns, json_4k 3,500 ns, '
    'truncated_json_4k 3,400 ns, form_encoded 182 ns, non_matching_json_4k 3,300 ns, plain_text 4.5 ns, '
    'URL query 235 ns, and 20 spans with 4 KB bodies 94,000 ns. '
    'Python capture also parses/serializes and truncates; Rust scans existing attribute strings and has broader coverage '
    '(in particular form bodies and truncated JSON). Differences therefore combine runtime overhead, work performed, and correctness coverage. '
    'Do not interpret the fast Python no-op paths as equivalent successful masking.', '']
for mode in r:
    val = r[mode]['shipped']['invocation20/json_4k']['p50_ns']
    lines += [f'For `{mode}`, Python 20-span capture p50 is **{val/1e6:.3f} ms** '
              f'({val/94000:.1f}× the supplied Rust 94 µs reference), with the scope/coverage differences above.','']
(HERE/'python.md').write_text('\n'.join(lines))
print(HERE/'python.md')
