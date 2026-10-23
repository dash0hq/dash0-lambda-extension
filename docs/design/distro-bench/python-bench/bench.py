"""Benchmark real distro code; run with PYTHONDONTWRITEBYTECODE=1 (see report)."""
import gc
import hashlib
import importlib.metadata
import json
import math
import os
from pathlib import Path
import platform
import statistics
import subprocess
import sys
import time
import tracemalloc
from urllib.parse import urlsplit

ROOT = Path(__file__).resolve().parents[3]
FIXTURE = Path('/Users/rm/code/dash0/dash0-claude/central-masking-extension/benches/fixtures/masking-payloads.json')
sys.path.insert(0, str(ROOT / 'opt/python/distro/src'))
os.environ['DASH0_SWITCH_OFF'] = 'true'
os.environ['DASH0_DISABLE_PYTHON_DEPENDENCY_CHECK'] = 'true'
os.environ['PYTHONDONTWRITEBYTECODE'] = '1'
os.environ['OTEL_ATTRIBUTE_VALUE_LENGTH_LIMIT'] = '4096'
os.environ['OTEL_SPAN_ATTRIBUTE_VALUE_LENGTH_LIMIT'] = '4096'
F = json.loads(FIXTURE.read_text())
MODES = {'default': F['default_patterns'], 'custom_email': F['custom_rule']}


def configure(mode):
    for key in list(os.environ):
        if key.startswith('LUMIGO_SECRET_MASKING_REGEX') or key == 'LUMIGO_BLACKLIST_REGEX' or key.startswith('DASH0_MASK_'):
            del os.environ[key]
    # Equivalent to wrapper map_masking_env: global custom rules replace defaults.
    os.environ['DASH0_MASK_RULES'] = json.dumps(MODES[mode])
    os.environ['LUMIGO_SECRET_MASKING_REGEX'] = os.environ['DASH0_MASK_RULES']


from dash0_opentelemetry.libs import masking as m
from dash0_opentelemetry.libs.json_utils import dump_with_context
from dash0_opentelemetry.instrumentations.instrumentation_utils import add_body_attribute

REAL_GET_RULE = m.get_rule


def quantiles(values):
    values = sorted(values)
    return {'p50_ns': statistics.median(values), 'p99_ns': values[math.ceil(len(values)*.99)-1]}


def measure(fn, n=20000):
    for _ in range(1500):
        fn()
    gc.collect()
    gc.disable()
    values = []
    try:
        for _ in range(n):
            t = time.perf_counter_ns()
            fn()
            values.append(time.perf_counter_ns()-t)
    finally:
        gc.enable()
    result = quantiles(values)
    result['samples'] = n
    peaks, retained = [], []
    tracemalloc.start()
    for _ in range(101):
        gc.collect()
        tracemalloc.reset_peak()
        before = tracemalloc.get_traced_memory()[0]
        out = fn()
        current, peak = tracemalloc.get_traced_memory()
        peaks.append(max(0, peak-before))
        retained.append(max(0, current-before))
        del out
    tracemalloc.stop()
    result['peak_traced_B'] = statistics.median(peaks)
    result['retained_B'] = statistics.median(retained)
    return result


class CaptureSpan:
    # Retains set_attribute outputs without SDK span construction/export overhead.
    def __init__(self):
        self.attributes = {}

    def set_attribute(self, key, value):
        self.attributes[key] = value


def span_fn(attrs):
    # Header JSON in the fixture represents the mappings Requests already has.
    # Deserialize once here, outside measurement, rather than charge capture for
    # decoding headers that the instrumentation already receives as mappings.
    req_headers = json.loads(attrs['http.request.headers'])
    res_headers = json.loads(attrs['http.response.headers'])
    others = {k: v for k, v in attrs.items() if k not in (
        'http.request.headers', 'http.response.headers', 'http.request.body', 'http.response.body')}

    def capture():
        span = CaptureSpan()
        span.attributes.update(others)  # Includes url.full unchanged, as in Requests hooks.
        span.set_attribute('http.request.headers', dump_with_context('requestHeaders', req_headers))
        add_body_attribute(span, attrs['http.request.body'], 'http.request.body', 'requestBody')
        span.set_attribute('http.response.headers', dump_with_context('responseHeaders', res_headers))
        add_body_attribute(span, attrs['http.response.body'], 'http.response.body', 'responseBody')
        return span.attributes
    return capture


def invocation_fn(capture):
    def invoke():
        return [capture() for _ in range(F['spans_per_invocation'])]
    return invoke


data = {'metadata': {'python': sys.version, 'platform': platform.platform(),
    'fixture_sha256': hashlib.sha256(FIXTURE.read_bytes()).hexdigest(),
    'fixture_bytes': {k:len(v.encode()) for k,v in F['payloads'].items()},
    'packages': {k:importlib.metadata.version(k) for k in ['lumigo-core', 'opentelemetry-sdk', 'opentelemetry-instrumentation-requests']},
    'git_head': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()},
    'results': {}, 'correctness': {}, 'cold': {}}
for mode in MODES:
    configure(mode)
    m.get_rule = REAL_GET_RULE
    m._parse_rule.cache_clear()
    cached = {context: REAL_GET_RULE(context) for context in [None, *m.CONTEXT_ENVS]}
    data['results'][mode] = {}
    data['correctness'][mode] = {
        'mask_text': {name:m.mask_text(value, 'requestBody', 4096) for name,value in F['payloads'].items()},
        'mask_and_dump': {name:m.mask_and_dump(value, 'requestBody', 4096) for name,value in F['payloads'].items()},
        'spans': {name:span_fn(attrs)() for name,attrs in F['span_attributes'].items()}}
    query = urlsplit(F['span_attributes']['small_json']['url.full']).query
    data['correctness'][mode]['query'] = dump_with_context('queryParams', m.parse_query_string(query))
    for lookup in ['shipped', 'cached_rule']:
        m.get_rule = REAL_GET_RULE if lookup == 'shipped' else lambda context=None: cached[context]
        rows = {}
        for name, value in F['payloads'].items():
            rows['mask_text/'+name] = measure(lambda value=value: m.mask_text(value, 'requestBody', 4096))
            rows['mask_and_dump/'+name] = measure(lambda value=value: m.mask_and_dump(value, 'requestBody', 4096))
        rows['query_parse_only'] = measure(lambda: m.parse_query_string(query))
        rows['query_parse_and_mask'] = measure(lambda: dump_with_context('queryParams', m.parse_query_string(query)))
        rows['get_rule_only'] = measure(lambda: m.get_rule('requestBody'))
        for name, attrs in F['span_attributes'].items():
            capture = span_fn(attrs)
            rows['span/'+name] = measure(capture, n=10000)
            rows['invocation20/'+name] = measure(invocation_fn(capture), n=2000)
        data['results'][mode][lookup] = rows
        print(mode, lookup, 'complete', flush=True)
    m.get_rule = REAL_GET_RULE
    data['cold'][mode] = {}
    for name in ['small_json', 'json_4k']:
        values = []
        for _ in range(30):
            proc = subprocess.run([sys.executable, str(Path(__file__).with_name('cold.py')), json.dumps(MODES[mode]), F['payloads'][name]], capture_output=True, text=True, check=True)
            values.append(json.loads(proc.stdout))
        data['cold'][mode][name] = {k:quantiles([v[k] for v in values]) for k in values[0]}
        data['cold'][mode][name]['samples'] = values
Path(__file__).with_name('results.json').write_text(json.dumps(data, indent=2))
print('Wrote', Path(__file__).with_name('results.json'))
