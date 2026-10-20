#!/usr/bin/env python3
"""Local performance harness for the Dash0 Lambda extension binary.

Measures cold-start latency, binary size, and event-loop retry behavior
WITHOUT deploying anything to AWS, by running the real release binary
against a minimal local mock of the Lambda Extensions/Runtime API
(mock_lambda_api.py). Meant for fast iteration while changing the startup
path; the authoritative, AWS-backed end-to-end numbers across all supported
language runtimes still come from ../benchmarks.

Caveat: this builds and runs whatever `cargo build --release` produces on
the host running it (e.g. native arm64/macOS), not the
aarch64-unknown-linux-musl binary that actually ships in the Lambda layer.
Binary-size deltas from a given change transfer directly. Cold-start
wall-clock numbers are only meaningful as a *relative*, before/after
comparison on the same machine -- not as an absolute prediction of real
Lambda cold-start latency.

Usage:
    bench.py build
    bench.py coldstart --runs 15 --label baseline --out results/baseline.json
    bench.py coldstart --runs 15 --secret-arn arn:aws:secretsmanager:us-west-2:123456789012:secret:x \\
             --label baseline-secrets --out results/baseline-secrets.json
    bench.py busyloop --duration 6 --label baseline --out results/baseline-busyloop.json
    bench.py compare --baseline results/baseline.json --candidate results/candidate.json
"""
from __future__ import annotations

import argparse
import http.server
import json
import os
import re
import select
import socket
import statistics
import subprocess
import sys
import threading
import time
from datetime import datetime, timezone
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from mock_lambda_api import serve as serve_mock  # noqa: E402

REPO_ROOT = Path(__file__).resolve().parent.parent
BINARY = REPO_ROOT / "target" / "release" / "aws-lambda-runtime-api-proxy-rs"
RESULTS_DIR = Path(__file__).resolve().parent / "results"

TIMESTAMP_RE = re.compile(r"^(?P<main>\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2})\.(?P<frac>\d+)Z$")


def parse_log_line(line: str) -> tuple[float, dict] | tuple[None, None]:
    """Parses one JSON tracing log line, returning (unix_epoch_seconds, obj)."""
    try:
        obj = json.loads(line)
        m = TIMESTAMP_RE.match(obj["timestamp"])
        if not m:
            return None, None
        frac = (m.group("frac") + "000000")[:6]
        dt = datetime.strptime(m.group("main"), "%Y-%m-%dT%H:%M:%S").replace(tzinfo=timezone.utc)
        return dt.timestamp() + int(frac) / 1_000_000, obj
    except Exception:
        return None, None


def free_port() -> int:
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    port = s.getsockname()[1]
    s.close()
    return port


def start_mock(port: int) -> http.server.ThreadingHTTPServer:
    server = serve_mock(port)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    return server


def wait_for_port(port: int, timeout_s: float = 3.0) -> None:
    """Blocks until something is accepting connections on 127.0.0.1:port.

    register() in this binary is fatal-on-failure (a panic, not a retry), so
    spawning it before a subprocess-based mock has actually bound its socket
    silently produces a false negative: the extension dies immediately,
    never reaches the code path under test, and looks "fine" (0% CPU) for
    the wrong reason.
    """
    deadline = time.time() + timeout_s
    while time.time() < deadline:
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=0.2):
                return
        except OSError:
            time.sleep(0.02)
    raise RuntimeError(f"mock server on port {port} never came up within {timeout_s}s")


def build() -> None:
    subprocess.run(["cargo", "build", "--release"], cwd=REPO_ROOT, check=True)


def binary_size() -> int:
    if not BINARY.exists():
        raise SystemExit(f"binary not found at {BINARY} -- run `bench.py build` first")
    return BINARY.stat().st_size


def base_env(runtime_api_port: int, secret_arn: str | None) -> dict:
    env = os.environ.copy()
    env["AWS_LAMBDA_RUNTIME_API"] = f"127.0.0.1:{runtime_api_port}"
    env["DASH0_EXTENSION_LOG_LEVEL"] = "info"
    env["DASH0_LISTENER_PORT"] = str(free_port())
    if secret_arn:
        env["DASH0_TOKEN_SECRET_ARN"] = secret_arn
        env.setdefault("AWS_ACCESS_KEY_ID", "AKIAFAKEFAKEFAKEFAKE")
        env.setdefault("AWS_SECRET_ACCESS_KEY", "fakefakefakefakefakefakefakefakefakefake")
    else:
        env["DASH0_TOKEN"] = "test-token"
    return env


def run_coldstart_trial(runtime_api_port: int, secret_arn: str | None, timeout_s: float = 5.0) -> dict:
    env = base_env(runtime_api_port, secret_arn)
    t0 = time.time()
    proc = subprocess.Popen(
        [str(BINARY)], env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True
    )
    listening_at = None
    registered_at = None
    deadline = time.time() + timeout_s
    try:
        while time.time() < deadline:
            remaining = deadline - time.time()
            ready, _, _ = select.select([proc.stdout], [], [], max(remaining, 0))
            if not ready:
                break
            line = proc.stdout.readline()
            if not line:
                break
            ts, obj = parse_log_line(line)
            if ts is None:
                continue
            msg = obj.get("message", "")
            if listening_at is None and "listening on" in msg:
                listening_at = ts
            if "Registered with accountId" in msg:
                registered_at = ts
                break
    finally:
        proc.kill()
        try:
            proc.wait(timeout=2)
        except subprocess.TimeoutExpired:
            pass

    if registered_at is None:
        raise RuntimeError("extension did not register within timeout -- check DASH0_LISTENER_PORT/env setup")

    return {
        "listen_ms": (listening_at - t0) * 1000 if listening_at else None,
        "register_ms": (registered_at - t0) * 1000,
    }


def summarize(values: list[float]) -> dict:
    sorted_vals = sorted(values)
    return {
        "min": sorted_vals[0],
        "max": sorted_vals[-1],
        "avg": statistics.mean(values),
        "median": statistics.median(values),
        "p95": sorted_vals[min(int(len(sorted_vals) * 0.95), len(sorted_vals) - 1)],
    }


def save(result: dict, out: str | None) -> Path:
    RESULTS_DIR.mkdir(exist_ok=True)
    path = Path(out) if out else RESULTS_DIR / f"{result['kind']}-{result['label']}.json"
    path.write_text(json.dumps(result, indent=2) + "\n")
    return path


def cmd_coldstart(args: argparse.Namespace) -> None:
    port = free_port()
    mock = start_mock(port)
    try:
        trials = [run_coldstart_trial(port, args.secret_arn) for _ in range(args.runs)]
    finally:
        mock.shutdown()

    register_ms = [t["register_ms"] for t in trials]
    result = {
        "kind": "coldstart",
        "label": args.label,
        "timestamp": datetime.now(timezone.utc).isoformat(),
        "runs": args.runs,
        "secret_arn": bool(args.secret_arn),
        "register_ms": summarize(register_ms),
        "binary_size_bytes": binary_size(),
        "raw_register_ms": register_ms,
    }
    path = save(result, args.out)
    print(f"saved -> {path}")
    print(json.dumps(result, indent=2))


def cmd_busyloop(args: argparse.Namespace) -> None:
    """Regression test for the get_next() no-backoff bug: kill the mock mid-run
    (simulating the Runtime API becoming briefly unreachable) and sample CPU%
    for `duration` seconds. Fixed code should settle near idle; unfixed code
    pegs a core indefinitely.

    The mock runs as a real subprocess, not an in-process thread: the
    extension already has a `get_next()` request in flight against it
    (that's the whole point of the long-poll), and only killing the mock's
    whole process resets that in-flight connection too. Closing just the
    listening socket (e.g. via ThreadingHTTPServer.server_close()) leaves
    already-accepted connections alone, so the extension would just keep
    waiting on the one it already has -- no error, no retry, 0% CPU, and a
    false "no bug here" reading."""
    port = free_port()
    mock_script = Path(__file__).resolve().parent / "mock_lambda_api.py"
    mock_proc = subprocess.Popen(
        [sys.executable, str(mock_script), str(port)], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL
    )
    wait_for_port(port)
    env = base_env(port, secret_arn=None)
    log_path = RESULTS_DIR / f".busyloop-{args.label}.log"
    RESULTS_DIR.mkdir(exist_ok=True)
    log_file = open(log_path, "w")
    proc = subprocess.Popen([str(BINARY)], env=env, stdout=log_file, stderr=subprocess.STDOUT)
    try:
        time.sleep(1.5)  # let it register and issue its first get_next() first
        log_file.flush()
        log_text = Path(log_path).read_text()
        if "Registered with accountId" not in log_text:
            raise RuntimeError(
                f"extension never registered before the mock was killed -- see {log_path}. "
                "This is a harness timing issue, not the bug under test: bump the sleep above."
            )
        mock_proc.kill()
        mock_proc.wait(timeout=2)

        samples = []
        end = time.time() + args.duration
        while time.time() < end:
            time.sleep(0.5)
            try:
                out = subprocess.check_output(["ps", "-o", "%cpu=", "-p", str(proc.pid)]).decode().strip()
                samples.append(float(out))
            except subprocess.CalledProcessError:
                break  # process exited
    finally:
        proc.kill()
        try:
            proc.wait(timeout=2)
        except subprocess.TimeoutExpired:
            pass
        if mock_proc.poll() is None:
            mock_proc.kill()
        log_file.close()

    result = {
        "kind": "busyloop",
        "label": args.label,
        "timestamp": datetime.now(timezone.utc).isoformat(),
        "duration_s": args.duration,
        "cpu_pct_samples": samples,
        "cpu_pct_avg": statistics.mean(samples) if samples else None,
        "cpu_pct_max": max(samples) if samples else None,
    }
    path = save(result, args.out)
    print(f"saved -> {path}")
    print(json.dumps(result, indent=2))


def cmd_compare(args: argparse.Namespace) -> None:
    a = json.loads(Path(args.baseline).read_text())
    b = json.loads(Path(args.candidate).read_text())
    if a["kind"] != b["kind"]:
        raise SystemExit(f"cannot compare {a['kind']} against {b['kind']}")

    print(f"{'':<28}{'baseline':>14}{'candidate':>14}{'delta':>16}")
    if a["kind"] == "coldstart":
        for k in ["min", "median", "avg", "p95", "max"]:
            av, bv = a["register_ms"][k], b["register_ms"][k]
            print(f"register_ms.{k:<15}{av:>14.2f}{bv:>14.2f}{bv - av:>+14.2f}ms")
        asz, bsz = a["binary_size_bytes"], b["binary_size_bytes"]
        pct = 100 * (bsz - asz) / asz
        print(f"{'binary_size_bytes':<28}{asz:>14}{bsz:>14}{bsz - asz:>+14} ({pct:+.1f}%)")
    elif a["kind"] == "busyloop":
        for k in ["cpu_pct_avg", "cpu_pct_max"]:
            av, bv = a[k], b[k]
            print(f"{k:<28}{av:>14.1f}{bv:>14.1f}{bv - av:>+14.1f}")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="cmd", required=True)

    b = sub.add_parser("build", help="cargo build --release")
    b.set_defaults(func=lambda _: build())

    cs = sub.add_parser("coldstart", help="measure spawn-to-registered latency + binary size")
    cs.add_argument("--runs", type=int, default=15)
    cs.add_argument("--label", default="run")
    cs.add_argument("--secret-arn", default=None, help="exercise the Secrets Manager token path")
    cs.add_argument("--out", default=None)
    cs.set_defaults(func=cmd_coldstart)

    bl = sub.add_parser(
        "busyloop",
        help="kill the mock mid-run and sample CPU%% -- regression test for the get_next() backoff fix",
    )
    bl.add_argument("--duration", type=int, default=6)
    bl.add_argument("--label", default="run")
    bl.add_argument("--out", default=None)
    bl.set_defaults(func=cmd_busyloop)

    cmp_ = sub.add_parser("compare", help="print a before/after table from two saved result files")
    cmp_.add_argument("--baseline", required=True)
    cmp_.add_argument("--candidate", required=True)
    cmp_.set_defaults(func=cmd_compare)

    args = parser.parse_args()
    args.func(args)


if __name__ == "__main__":
    main()
