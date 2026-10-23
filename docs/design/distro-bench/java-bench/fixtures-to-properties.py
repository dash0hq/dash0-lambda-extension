# Converts the shared fixtures into base64 .properties so the cold run doesn't preload Jackson.
import json, base64, sys
d = json.load(open(sys.argv[1]))
b = lambda s: base64.b64encode(s.encode()).decode()
out = [f"payload.{k}={b(v)}" for k, v in d["payloads"].items()]
for s, attrs in d["span_attributes"].items():
    out += [f"span.{s}.{k}={b(v)}" for k, v in attrs.items()]
out += [f"custom_rule={b(json.dumps(d['custom_rule']))}", f"spans_per_invocation={d['spans_per_invocation']}"]
open(sys.argv[2], "w").write("\n".join(out) + "\n")
