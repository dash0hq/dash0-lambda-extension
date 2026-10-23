// Distro masking vs extension masking on real Lambda. Invokes every function of
// MaskingBenchmarkStack (../iac/lib/masking-benchmark-stack.ts), then reads the
// platform.report / platform.runtimeDone events from CloudWatch Logs per request.
//
//   npx tsx run-masking-benchmark.ts             run, write results JSON, print the tables
//   npx tsx run-masking-benchmark.ts <file.json>  print the tables of an earlier run
import {
  LambdaClient,
  InvokeCommand,
  GetFunctionConfigurationCommand,
  UpdateFunctionConfigurationCommand,
  waitUntilFunctionUpdated,
} from "@aws-sdk/client-lambda";
import {
  CloudWatchLogsClient,
  FilterLogEventsCommand,
} from "@aws-sdk/client-cloudwatch-logs";
import { writeFileSync, readFileSync, mkdirSync } from "fs";
import { resolve, dirname } from "path";
import { fileURLToPath } from "url";

const PREFIX = process.env.RESOURCE_PREFIX ?? "cmask-bench-";
const REGION = process.env.AWS_REGION ?? "us-west-2";
const COLD = parseInt(process.env.COLD_START_INVOCATIONS ?? "10", 10);
const WARM = parseInt(process.env.WARM_INVOCATIONS ?? "50", 10);
const CONCURRENCY = parseInt(process.env.BENCHMARK_CONCURRENCY ?? "9", 10);
const RUNTIMES = ["node22", "python313", "java21"];
const VARIANTS = ["baseline", "distro", "ext", "none"];
const MEMORY_SIZES = (process.env.MASKING_MEMORY_SIZES ?? "128,512,1024").split(",");

const __dirname = dirname(fileURLToPath(import.meta.url));
const OUTPUT_FILE =
  process.env.OUTPUT_FILE ??
  resolve(__dirname, "..", "results", `${new Date().toISOString().slice(0, 10)}-masking-aws.json`);

const EVENT = JSON.stringify({ orderId: 42, customer: { name: "Jane Doe", password: "pw-SECRETVALUE-event" } });

const lambdaClient = new LambdaClient({ region: REGION });
const logsClient = new CloudWatchLogsClient({ region: REGION });

interface Invocation {
  phase: "cold" | "warm";
  requestId: string;
  clientMs: number;
  error?: string;
  initMs?: number;
  durationMs?: number;
  billedMs?: number;
  maxMemoryMb?: number;
  runtimeDoneMs?: number;
  // platform.runtimeDone spans: until the runtime posted the response, and from there to /next
  responseLatencyMs?: number;
  runtimeOverheadMs?: number;
  // platform.report span: what extensions do after the runtime is done
  extensionOverheadMs?: number;
}

interface FunctionRun {
  functionName: string;
  runtime: string;
  variant: string;
  memoryMb: number;
  invocations: Invocation[];
}

async function invoke(functionName: string, phase: Invocation["phase"]): Promise<Invocation> {
  const start = performance.now();
  const res = await lambdaClient.send(
    new InvokeCommand({ FunctionName: functionName, Payload: Buffer.from(EVENT) })
  );
  const clientMs = performance.now() - start;
  const inv: Invocation = { phase, requestId: res.$metadata.requestId ?? "", clientMs };
  if (res.FunctionError) inv.error = Buffer.from(res.Payload ?? []).toString().slice(0, 300);
  return inv;
}

async function forceColdStart(functionName: string): Promise<void> {
  const config = await lambdaClient.send(new GetFunctionConfigurationCommand({ FunctionName: functionName }));
  await lambdaClient.send(
    new UpdateFunctionConfigurationCommand({
      FunctionName: functionName,
      Environment: { Variables: { ...config.Environment?.Variables, BENCHMARK_TIMESTAMP: Date.now().toString() } },
    })
  );
  await waitUntilFunctionUpdated({ client: lambdaClient, maxWaitTime: 120 }, { FunctionName: functionName });
}

async function runFunction(run: FunctionRun): Promise<void> {
  for (let i = 0; i < COLD; i++) {
    await forceColdStart(run.functionName);
    run.invocations.push(await invoke(run.functionName, "cold"));
  }
  await invoke(run.functionName, "warm"); // warm-up, discarded
  for (let i = 0; i < WARM; i++) {
    run.invocations.push(await invoke(run.functionName, "warm"));
  }
  const errors = run.invocations.filter((i) => i.error).length;
  console.log(`${run.functionName}: done${errors ? `, ${errors} errors: ${run.invocations.find((i) => i.error)?.error}` : ""}`);
}

async function attachPlatformMetrics(run: FunctionRun, startTime: number): Promise<void> {
  const byId = new Map(run.invocations.map((i) => [i.requestId, i]));
  let nextToken: string | undefined;
  do {
    const page = await logsClient.send(
      new FilterLogEventsCommand({
        logGroupName: `/aws/lambda/${run.functionName}`,
        startTime,
        filterPattern: '{ $.type = "platform.report" || $.type = "platform.runtimeDone" }',
        nextToken,
      })
    );
    for (const e of page.events ?? []) {
      const rec = JSON.parse(e.message ?? "{}");
      const inv = byId.get(rec.record?.requestId);
      if (!inv) continue;
      const m = rec.record.metrics ?? {};
      const span = (name: string) => rec.record.spans?.find((x: { name: string }) => x.name === name)?.durationMs;
      if (rec.type === "platform.report") {
        inv.durationMs = m.durationMs;
        inv.billedMs = m.billedDurationMs;
        inv.maxMemoryMb = m.maxMemoryUsedMB;
        inv.initMs = m.initDurationMs;
        inv.extensionOverheadMs = span("extensionOverhead");
      } else {
        inv.runtimeDoneMs = m.durationMs;
        inv.responseLatencyMs = span("responseLatency");
        inv.runtimeOverheadMs = span("runtimeOverhead");
      }
    }
    nextToken = page.nextToken;
  } while (nextToken);
}

async function pool<T>(items: T[], n: number, fn: (item: T) => Promise<void>): Promise<void> {
  const queue = [...items];
  await Promise.all(
    Array.from({ length: n }, async () => {
      for (let item = queue.shift(); item; item = queue.shift()) await fn(item);
    })
  );
}

// ── Summary ────────────────────────────────────────────────────────────

function pct(values: number[], p: number): number | undefined {
  if (values.length === 0) return undefined;
  const sorted = [...values].sort((a, b) => a - b);
  return sorted[Math.min(sorted.length - 1, Math.ceil((p / 100) * sorted.length) - 1)];
}

function fmt(values: number[]): string {
  if (values.length === 0) return "–";
  return [50, 90, 99].map((p) => pct(values, p)!.toFixed(1)).join(" / ");
}

const METRICS: [string, keyof Invocation][] = [
  ["client", "clientMs"],
  ["responseLatency", "responseLatencyMs"],
  ["runtimeOverhead", "runtimeOverheadMs"],
  ["extensionOverhead", "extensionOverheadMs"],
  ["duration", "durationMs"],
  ["billed", "billedMs"],
];

function values(run: FunctionRun, phase: Invocation["phase"], key: keyof Invocation): number[] {
  return run.invocations
    .filter((i) => i.phase === phase && !i.error && typeof i[key] === "number")
    .map((i) => i[key] as number);
}

function printTables(runs: FunctionRun[]): void {
  for (const runtime of RUNTIMES) {
    for (const mem of MEMORY_SIZES) {
      const rows = runs.filter((r) => r.runtime === runtime && r.memoryMb === Number(mem));
      if (rows.length === 0) continue;
      console.log(`\n### ${runtime}, ${mem} MB (p50 / p90 / p99, ms)\n`);
      for (const phase of ["cold", "warm"] as const) {
        const cols = phase === "cold" ? [["init", "initMs"] as [string, keyof Invocation], ...METRICS] : METRICS;
        console.log(`| ${phase} | n | ${cols.map((c) => c[0]).join(" | ")} | max mem MB |`);
        console.log(`|---|---:|${cols.map(() => "---:").join("|")}|---:|`);
        for (const variant of VARIANTS) {
          const run = rows.find((r) => r.variant === variant);
          if (!run) continue;
          const n = run.invocations.filter((i) => i.phase === phase && !i.error).length;
          const mem = pct(values(run, phase, "maxMemoryMb"), 50);
          console.log(`| ${variant} | ${n} | ${cols.map(([, k]) => fmt(values(run, phase, k))).join(" | ")} | ${mem ?? "–"} |`);
        }
        console.log("");
      }
    }
  }
}

// ── Main ───────────────────────────────────────────────────────────────

async function main(): Promise<void> {
  if (process.argv[2]) {
    printTables(JSON.parse(readFileSync(process.argv[2], "utf-8")).runs);
    return;
  }
  const runs: FunctionRun[] = [];
  for (const memoryMb of MEMORY_SIZES.map(Number)) {
    for (const runtime of RUNTIMES) {
      for (const variant of VARIANTS) {
        runs.push({ functionName: `${PREFIX}${runtime}-${variant}-${memoryMb}`, runtime, variant, memoryMb, invocations: [] });
      }
    }
  }
  const startTime = Date.now() - 60_000;
  console.log(`Benchmarking ${runs.length} functions: ${COLD} cold + ${WARM} warm each, concurrency ${CONCURRENCY}`);
  await pool(runs, CONCURRENCY, runFunction);

  console.log("Waiting 60 s for CloudWatch Logs ingestion...");
  await new Promise((r) => setTimeout(r, 60_000));
  await pool(runs, 4, (run) => attachPlatformMetrics(run, startTime));
  const missing = runs.flatMap((r) => r.invocations).filter((i) => i.durationMs === undefined).length;
  if (missing) console.log(`WARNING: ${missing} invocations without platform.report`);

  mkdirSync(dirname(OUTPUT_FILE), { recursive: true });
  writeFileSync(
    OUTPUT_FILE,
    JSON.stringify({ date: new Date().toISOString(), region: REGION, cold: COLD, warm: WARM, runs }, null, 1)
  );
  console.log(`Results written to ${OUTPUT_FILE}`);
  printTables(runs);
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
