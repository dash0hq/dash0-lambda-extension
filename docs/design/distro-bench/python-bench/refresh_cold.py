"""Re-measure only cold imports using the minimal child, preserving warm samples."""
import json
import math
import os
from pathlib import Path
import statistics
import subprocess
import sys

here = Path(__file__).resolve().parent
data = json.loads((here/'results.json').read_text())
fixtures = json.loads(Path('/Users/rm/code/dash0/dash0-claude/central-masking-extension/benches/fixtures/masking-payloads.json').read_text())
for mode, rules in [('default', fixtures['default_patterns']), ('custom_email', fixtures['custom_rule'])]:
    data['cold'][mode] = {}
    for name in ['small_json', 'json_4k']:
        samples = []
        for _ in range(30):
            env = {key:value for key,value in os.environ.items() if not key.startswith('LUMIGO_SECRET_MASKING_REGEX') and key != 'LUMIGO_BLACKLIST_REGEX'}
            env['PYTHONDONTWRITEBYTECODE'] = '1'
            proc = subprocess.run([sys.executable, str(here/'cold.py'), json.dumps(rules), fixtures['payloads'][name]], env=env, capture_output=True, text=True, check=True)
            samples.append(json.loads(proc.stdout))
        row = {}
        for interval in samples[0]:
            values = sorted(s[interval] for s in samples)
            row[interval] = {'p50_ns':statistics.median(values), 'p99_ns':values[math.ceil(len(values)*.99)-1]}
        row['samples'] = samples
        data['cold'][mode][name] = row
        print(mode, name, row['total_ns'], flush=True)
(here/'results.json').write_text(json.dumps(data, indent=2))
