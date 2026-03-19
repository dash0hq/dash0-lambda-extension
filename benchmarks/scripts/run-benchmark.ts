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
  `results/${new Date().toISOString().slice(0, 10)}.md`;

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

function fmt(v: number | undefined, suffix: string): string {
  return v != null ? `${v}${suffix}` : "N/A";
}

function runtimeDisplayName(rt: string): string {
  return rt
    .replace("python3-", "Python 3.")
    .replace("nodejs", "Node.js ")
    .replace("-x", ".x")
    .replace("java", "Java ");
}

function generateMarkdown(summaries: Record<string, FunctionSummary>): string {
  const lines: string[] = [];
  const date = new Date().toISOString().slice(0, 10);

  lines.push(`# Benchmark Results - ${date}`);
  lines.push("");
  lines.push(`- **Region:** ${REGION}`);
  lines.push(`- **Cold start invocations per function:** ${COLD_START_INVOCATIONS}`);
  lines.push("");

  // Group runtimes by language
  const groups: { name: string; runtimes: string[] }[] = [
    { name: "Python", runtimes: PYTHON_RUNTIMES },
    { name: "Node.js", runtimes: NODE_RUNTIMES },
    { name: "Java", runtimes: JAVA_RUNTIMES },
  ];

  // Overview table
  lines.push("## Overview");
  lines.push("");
  lines.push("| Runtime | Baseline Init (avg) | Instrumented Init (avg) | Init Overhead | Baseline Memory (avg) | Instrumented Memory (avg) | Memory Overhead |");
  lines.push("| :------ | ------------------: | ----------------------: | ------------: | --------------------: | ------------------------: | --------------: |");

  for (const rt of ALL_RUNTIMES) {
    const b = summaries[`${PREFIX}bench-baseline-${rt}`];
    const i = summaries[`${PREFIX}bench-instrumented-${rt}`];

    const bInit = b?.initDurationMs?.avg;
    const iInit = i?.initDurationMs?.avg;
    const initOverhead =
      bInit != null && iInit != null
        ? `+${Math.round((iInit - bInit) * 10) / 10} ms`
        : "N/A";

    const bMem = b?.maxMemoryUsedMb?.avg;
    const iMem = i?.maxMemoryUsedMb?.avg;
    const memOverhead =
      bMem != null && iMem != null
        ? `+${Math.round((iMem - bMem) * 10) / 10} MB`
        : "N/A";

    lines.push(
      `| ${runtimeDisplayName(rt)} | ${fmt(bInit, " ms")} | ${fmt(iInit, " ms")} | ${initOverhead} | ${fmt(bMem, " MB")} | ${fmt(iMem, " MB")} | ${memOverhead} |`
    );
  }

  lines.push("");

  // Detailed tables per language
  for (const group of groups) {
    lines.push(`## ${group.name}`);
    lines.push("");
    lines.push("### Init Duration (ms)");
    lines.push("");
    lines.push("| Runtime | Type | Min | Avg | Median | P95 | Max | Samples |");
    lines.push("| :------ | :--- | --: | --: | -----: | --: | --: | ------: |");

    for (const rt of group.runtimes) {
      const displayName = runtimeDisplayName(rt);

      const b = summaries[`${PREFIX}bench-baseline-${rt}`];
      const i = summaries[`${PREFIX}bench-instrumented-${rt}`];

      if (b?.initDurationMs) {
        const d = b.initDurationMs;
        lines.push(
          `| ${displayName} | Baseline | ${d.min} | ${d.avg} | ${d.median} | ${d.p95} | ${d.max} | ${d.values.length} |`
        );
      }
      if (i?.initDurationMs) {
        const d = i.initDurationMs;
        lines.push(
          `| ${displayName} | Instrumented | ${d.min} | ${d.avg} | ${d.median} | ${d.p95} | ${d.max} | ${d.values.length} |`
        );
      }
    }

    lines.push("");
    lines.push("### Memory Usage (MB)");
    lines.push("");
    lines.push("| Runtime | Type | Min | Avg | Max | Samples |");
    lines.push("| :------ | :--- | --: | --: | --: | ------: |");

    for (const rt of group.runtimes) {
      const displayName = runtimeDisplayName(rt);

      const b = summaries[`${PREFIX}bench-baseline-${rt}`];
      const i = summaries[`${PREFIX}bench-instrumented-${rt}`];

      if (b?.maxMemoryUsedMb) {
        const m = b.maxMemoryUsedMb;
        lines.push(
          `| ${displayName} | Baseline | ${m.min} | ${m.avg} | ${m.max} | ${m.values.length} |`
        );
      }
      if (i?.maxMemoryUsedMb) {
        const m = i.maxMemoryUsedMb;
        lines.push(
          `| ${displayName} | Instrumented | ${m.min} | ${m.avg} | ${m.max} | ${m.values.length} |`
        );
      }
    }

    lines.push("");
  }

  return lines.join("\n");
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

  // Generate markdown report
  const markdown = generateMarkdown(summaries);
  console.log();
  console.log(markdown);

  // Save results
  mkdirSync(resolve(__dirname, "..", dirname(OUTPUT_FILE)), { recursive: true });
  const outputPath = resolve(__dirname, "..", OUTPUT_FILE);
  writeFileSync(outputPath, markdown + "\n");
  console.log();
  console.log(`Results saved to ${outputPath}`);
  console.log();
  console.log("=== Benchmark complete ===");
}

main().catch((err) => {
  console.error("Fatal error:", err);
  process.exit(1);
});
