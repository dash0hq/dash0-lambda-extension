import {
  LambdaClient,
  InvokeCommand,
  GetFunctionConfigurationCommand,
  UpdateFunctionConfigurationCommand,
  GetFunctionCommand,
  waitUntilFunctionUpdated,
} from "@aws-sdk/client-lambda";
import { writeFileSync, mkdirSync } from "fs";
import { resolve, dirname } from "path";
import { fileURLToPath } from "url";

// ── Configuration ──────────────────────────────────────────────────────

const PREFIX = process.env.RESOURCE_PREFIX ?? "";
const REGION = process.env.AWS_REGION ?? "us-west-2";
const COLD_START_INVOCATIONS = parseInt(
  process.env.COLD_START_INVOCATIONS ?? "10",
  10
);
const OUTPUT_FILE =
  process.env.OUTPUT_FILE ??
  `results/${new Date().toISOString().slice(0, 10)}.json`;

const CONCURRENCY = parseInt(process.env.BENCHMARK_CONCURRENCY ?? "10", 10);

const __dirname = dirname(fileURLToPath(import.meta.url));
const resultsDir = resolve(__dirname, "..", "results");

const PYTHON_RUNTIMES = [
  "python3-10",
  "python3-11",
  "python3-12",
  "python3-13",
  "python3-14",
];
const NODE_RUNTIMES = ["nodejs20-x", "nodejs22-x", "nodejs24-x"];
const JAVA_RUNTIMES = ["java17", "java21", "java25"];

const ALL_RUNTIMES = [...PYTHON_RUNTIMES, ...NODE_RUNTIMES, ...JAVA_RUNTIMES];

// ── Types ──────────────────────────────────────────────────────────────

interface InvocationResult {
  initDurationMs?: number;
  maxMemoryUsedMb?: number;
}

interface FunctionResults {
  functionName: string;
  invocations: InvocationResult[];
}

interface FunctionSummary {
  coldStartCount: number;
  invocationCount: number;
  initDurationMs?: {
    min: number;
    max: number;
    avg: number;
    median: number;
    p95: number;
    values: number[];
  };
  maxMemoryUsedMb?: {
    min: number;
    max: number;
    avg: number;
    values: number[];
  };
}

// ── Helpers ────────────────────────────────────────────────────────────

const lambdaClient = new LambdaClient({ region: REGION });

function parseReportLine(logResult: string | undefined): InvocationResult {
  if (!logResult) return {};

  const decoded = Buffer.from(logResult, "base64").toString("utf-8");
  const result: InvocationResult = {};

  const initMatch = decoded.match(/Init Duration:\s+([\d.]+)\s+ms/);
  if (initMatch) {
    result.initDurationMs = parseFloat(initMatch[1]);
  }

  const memMatch = decoded.match(/Max Memory Used:\s+(\d+)\s+MB/);
  if (memMatch) {
    result.maxMemoryUsedMb = parseInt(memMatch[1], 10);
  }

  return result;
}

async function forceColdStartAndInvoke(
  functionName: string
): Promise<InvocationResult> {
  // Get current environment
  const config = await lambdaClient.send(
    new GetFunctionConfigurationCommand({ FunctionName: functionName })
  );

  const currentVars = config.Environment?.Variables ?? {};
  const newVars = {
    ...currentVars,
    BENCHMARK_TIMESTAMP: Date.now().toString(),
  };

  // Update env var to force cold start
  await lambdaClient.send(
    new UpdateFunctionConfigurationCommand({
      FunctionName: functionName,
      Environment: { Variables: newVars },
    })
  );

  // Wait for update to complete
  await waitUntilFunctionUpdated(
    { client: lambdaClient, maxWaitTime: 60 },
    { FunctionName: functionName }
  );

  // Invoke with Tail to get REPORT in LogResult
  const response = await lambdaClient.send(
    new InvokeCommand({
      FunctionName: functionName,
      Payload: Buffer.from("{}"),
      LogType: "Tail",
    })
  );

  return parseReportLine(response.LogResult);
}

async function benchmarkFunction(
  functionName: string
): Promise<FunctionResults> {
  const invocations: InvocationResult[] = [];

  for (let i = 0; i < COLD_START_INVOCATIONS; i++) {
    try {
      const result = await forceColdStartAndInvoke(functionName);
      invocations.push(result);

      const init = result.initDurationMs
        ? `init=${result.initDurationMs}ms`
        : "";
      const mem = result.maxMemoryUsedMb
        ? `mem=${result.maxMemoryUsedMb}MB`
        : "";
      const tag = result.initDurationMs ? "cold" : "warm";
      console.log(
        `  [${functionName}] ${i + 1}/${COLD_START_INVOCATIONS} (${tag}) ${init} ${mem}`
      );
    } catch (err) {
      console.error(
        `  [${functionName}] ${i + 1}/${COLD_START_INVOCATIONS} ERROR: ${err}`
      );
    }
  }

  return { functionName, invocations };
}

function summarize(results: FunctionResults[]): Record<string, FunctionSummary> {
  const summaries: Record<string, FunctionSummary> = {};

  for (const { functionName, invocations } of results) {
    if (invocations.length === 0) continue;

    const coldStarts = invocations.filter((i) => i.initDurationMs != null);
    const allMem = invocations
      .filter((i) => i.maxMemoryUsedMb != null)
      .map((i) => i.maxMemoryUsedMb!);

    const summary: FunctionSummary = {
      coldStartCount: coldStarts.length,
      invocationCount: invocations.length,
    };

    if (coldStarts.length > 0) {
      const values = coldStarts.map((c) => c.initDurationMs!).sort((a, b) => a - b);
      summary.initDurationMs = {
        min: values[0],
        max: values[values.length - 1],
        avg: Math.round((values.reduce((a, b) => a + b, 0) / values.length) * 100) / 100,
        median: values[Math.floor(values.length / 2)],
        p95: values[Math.floor(values.length * 0.95)],
        values,
      };
    }

    if (allMem.length > 0) {
      const sorted = [...allMem].sort((a, b) => a - b);
      summary.maxMemoryUsedMb = {
        min: sorted[0],
        max: sorted[sorted.length - 1],
        avg: Math.round((sorted.reduce((a, b) => a + b, 0) / sorted.length) * 100) / 100,
        values: sorted,
      };
    }

    summaries[functionName] = summary;
  }

  return summaries;
}

function printTable(summaries: Record<string, FunctionSummary>) {
  const header = [
    "Runtime".padEnd(15),
    "Baseline Init".padStart(14),
    "Instr. Init".padStart(14),
    "Overhead".padStart(10),
    "Base Mem".padStart(10),
    "Instr Mem".padStart(10),
    "Mem Δ".padStart(8),
  ].join(" | ");

  const separator = header.replace(/[^|]/g, "-");

  console.log();
  console.log(header);
  console.log(separator);

  for (const rt of ALL_RUNTIMES) {
    const baselineKey = `${PREFIX}bench-baseline-${rt}`;
    const instrKey = `${PREFIX}bench-instrumented-${rt}`;

    const b = summaries[baselineKey];
    const i = summaries[instrKey];

    const bInit = b?.initDurationMs?.avg;
    const iInit = i?.initDurationMs?.avg;
    const overhead =
      bInit != null && iInit != null
        ? `${Math.round((iInit - bInit) * 10) / 10}ms`
        : "N/A";

    const bMem = b?.maxMemoryUsedMb?.avg;
    const iMem = i?.maxMemoryUsedMb?.avg;
    const memOverhead =
      bMem != null && iMem != null
        ? `${Math.round((iMem - bMem) * 10) / 10}MB`
        : "N/A";

    const fmt = (v: number | undefined, suffix: string) =>
      v != null ? `${v}${suffix}` : "N/A";

    console.log(
      [
        rt.padEnd(15),
        fmt(bInit, "ms").padStart(14),
        fmt(iInit, "ms").padStart(14),
        overhead.padStart(10),
        fmt(bMem, "MB").padStart(10),
        fmt(iMem, "MB").padStart(10),
        memOverhead.padStart(8),
      ].join(" | ")
    );
  }

  console.log();
}

// Run tasks with limited concurrency
async function runWithConcurrency<T>(
  tasks: (() => Promise<T>)[],
  concurrency: number
): Promise<T[]> {
  const results: T[] = new Array(tasks.length);
  let nextIndex = 0;

  async function worker() {
    while (nextIndex < tasks.length) {
      const index = nextIndex++;
      results[index] = await tasks[index]();
    }
  }

  const workers = Array.from(
    { length: Math.min(concurrency, tasks.length) },
    () => worker()
  );
  await Promise.all(workers);
  return results;
}

// ── Main ───────────────────────────────────────────────────────────────

async function main() {
  const allFunctions = ALL_RUNTIMES.flatMap((rt) => [
    `${PREFIX}bench-baseline-${rt}`,
    `${PREFIX}bench-instrumented-${rt}`,
  ]);

  console.log("=== Benchmark: Collecting cold start metrics ===");
  console.log(`Region: ${REGION}`);
  console.log(`Cold start invocations per function: ${COLD_START_INVOCATIONS}`);
  console.log(`Concurrency: ${CONCURRENCY}`);
  console.log(`Functions: ${allFunctions.length}`);
  console.log();

  // Verify first function exists
  const firstFunc = allFunctions[0];
  try {
    await lambdaClient.send(
      new GetFunctionCommand({ FunctionName: firstFunc })
    );
    console.log(`Verified function exists: ${firstFunc}`);
  } catch {
    console.error(`ERROR: Could not find function ${firstFunc}`);
    console.error(
      "Make sure the benchmark stack is deployed: cd benchmarks/iac && npx cdk deploy"
    );
    process.exit(1);
  }

  console.log();

  // Run all functions in parallel (with concurrency limit)
  const tasks = allFunctions.map(
    (fn) => () => benchmarkFunction(fn)
  );

  const results = await runWithConcurrency(tasks, CONCURRENCY);
  const summaries = summarize(results);

  // Print comparison table
  printTable(summaries);

  // Save results
  mkdirSync(resolve(__dirname, "..", dirname(OUTPUT_FILE)), { recursive: true });
  const outputPath = resolve(__dirname, "..", OUTPUT_FILE);

  const output = {
    timestamp: new Date().toISOString(),
    region: REGION,
    coldStartInvocations: COLD_START_INVOCATIONS,
    functions: summaries,
  };

  writeFileSync(outputPath, JSON.stringify(output, null, 2));
  console.log(`Results saved to ${outputPath}`);
  console.log();
  console.log("=== Benchmark complete ===");
}

main().catch((err) => {
  console.error("Fatal error:", err);
  process.exit(1);
});
