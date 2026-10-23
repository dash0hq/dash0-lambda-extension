"""Fresh-process measurement with no benchmark library imports before the timer."""
import os
import sys
import time

sys.path.insert(0, os.path.join(os.path.dirname(__file__), '../distro/src'))
os.environ['DASH0_SWITCH_OFF'] = 'true'
os.environ['DASH0_DISABLE_PYTHON_DEPENDENCY_CHECK'] = 'true'
os.environ['LUMIGO_SECRET_MASKING_REGEX'] = sys.argv[1]
t0 = time.perf_counter_ns()
from dash0_opentelemetry.libs import masking as m
t1 = time.perf_counter_ns()
result = m.mask_text(sys.argv[2], 'requestBody', 4096)
t2 = time.perf_counter_ns()
print(m.json.dumps({'import_ns':t1-t0, 'first_call_ns':t2-t1, 'total_ns':t2-t0}))
