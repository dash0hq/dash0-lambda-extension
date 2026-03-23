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

// Extension-only (manual layer, no auto-instrumentation)
const EXTENSION_ONLY_FUNCTION = "nodejs24-x";

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
  lines.push("## Init Duration Overview (avg, ms)");
  lines.push("");
  lines.push("| Runtime | Baseline | Dash0 | Dash0 Overhead | OSS OTel | OSS OTel Overhead |");
  lines.push("| :------ | -------: | ----: | -------------: | -------: | ----------------: |");

  for (const rt of ALL_RUNTIMES) {
    const b = summaries[`${PREFIX}bench-baseline-${rt}`];
    const d = summaries[`${PREFIX}bench-instrumented-${rt}`];
    const o = summaries[`${PREFIX}bench-oss-otel-${rt}`];

    const bInit = b?.initDurationMs?.avg;
    const dInit = d?.initDurationMs?.avg;
    const oInit = o?.initDurationMs?.avg;

    const dOverhead = bInit != null && dInit != null ? `+${Math.round((dInit - bInit) * 10) / 10} ms` : "N/A";
    const oOverhead = bInit != null && oInit != null ? `+${Math.round((oInit - bInit) * 10) / 10} ms` : "N/A";

    lines.push(
      `| ${runtimeDisplayName(rt)} | ${fmt(bInit, " ms")} | ${fmt(dInit, " ms")} | ${dOverhead} | ${fmt(oInit, " ms")} | ${oOverhead} |`
    );
  }

  lines.push("");

  lines.push("## Memory Overview (avg, MB)");
  lines.push("");
  lines.push("| Runtime | Baseline | Dash0 | Dash0 Overhead | OSS OTel | OSS OTel Overhead |");
  lines.push("| :------ | -------: | ----: | -------------: | -------: | ----------------: |");

  for (const rt of ALL_RUNTIMES) {
    const b = summaries[`${PREFIX}bench-baseline-${rt}`];
    const d = summaries[`${PREFIX}bench-instrumented-${rt}`];
    const o = summaries[`${PREFIX}bench-oss-otel-${rt}`];

    const bMem = b?.maxMemoryUsedMb?.avg;
    const dMem = d?.maxMemoryUsedMb?.avg;
    const oMem = o?.maxMemoryUsedMb?.avg;

    const dOverhead = bMem != null && dMem != null ? `+${Math.round((dMem - bMem) * 10) / 10} MB` : "N/A";
    const oOverhead = bMem != null && oMem != null ? `+${Math.round((oMem - bMem) * 10) / 10} MB` : "N/A";

    lines.push(
      `| ${runtimeDisplayName(rt)} | ${fmt(bMem, " MB")} | ${fmt(dMem, " MB")} | ${dOverhead} | ${fmt(oMem, " MB")} | ${oOverhead} |`
    );
  }

  lines.push("");

  // Extension-only section
  const extOnly = summaries[`${PREFIX}bench-extension-only-${EXTENSION_ONLY_FUNCTION}`];
  const nodeBaseline = summaries[`${PREFIX}bench-baseline-${EXTENSION_ONLY_FUNCTION}`];
  const nodeInstr = summaries[`${PREFIX}bench-instrumented-${EXTENSION_ONLY_FUNCTION}`];

  lines.push("## Extension-Only Overhead (No Auto-Instrumentation)");
  lines.push("");
  lines.push(`Comparison using Node.js 24.x with the manual layer (extension only, no distro instrumentation):`);
  lines.push("");
  lines.push("| Variant | Init (avg) | Init Overhead | Memory (avg) | Memory Overhead |");
  lines.push("| :------ | ---------: | ------------: | -----------: | --------------: |");

  const baseInit = nodeBaseline?.initDurationMs?.avg;
  const baseMem = nodeBaseline?.maxMemoryUsedMb?.avg;

  const extInit = extOnly?.initDurationMs?.avg;
  const extMem = extOnly?.maxMemoryUsedMb?.avg;
  const extInitOh = baseInit != null && extInit != null ? `+${Math.round((extInit - baseInit) * 10) / 10} ms` : "N/A";
  const extMemOh = baseMem != null && extMem != null ? `+${Math.round((extMem - baseMem) * 10) / 10} MB` : "N/A";

  const instrInit = nodeInstr?.initDurationMs?.avg;
  const instrMem = nodeInstr?.maxMemoryUsedMb?.avg;
  const instrInitOh = baseInit != null && instrInit != null ? `+${Math.round((instrInit - baseInit) * 10) / 10} ms` : "N/A";
  const instrMemOh = baseMem != null && instrMem != null ? `+${Math.round((instrMem - baseMem) * 10) / 10} MB` : "N/A";

  lines.push(`| Baseline (no layer) | ${fmt(baseInit, " ms")} | - | ${fmt(baseMem, " MB")} | - |`);
  lines.push(`| Extension only (manual layer) | ${fmt(extInit, " ms")} | ${extInitOh} | ${fmt(extMem, " MB")} | ${extMemOh} |`);
  lines.push(`| Full instrumentation (node layer) | ${fmt(instrInit, " ms")} | ${instrInitOh} | ${fmt(instrMem, " MB")} | ${instrMemOh} |`);
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
      const d = summaries[`${PREFIX}bench-instrumented-${rt}`];
      const o = summaries[`${PREFIX}bench-oss-otel-${rt}`];

      const variants: [string, FunctionSummary | undefined][] = [["Baseline", b], ["Dash0", d], ["OSS OTel", o]];
      for (const [label, s] of variants) {
        if (s?.initDurationMs) {
          const v = s.initDurationMs;
          lines.push(
            `| ${displayName} | ${label} | ${v.min} | ${v.avg} | ${v.median} | ${v.p95} | ${v.max} | ${v.values.length} |`
          );
        }
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
      const d = summaries[`${PREFIX}bench-instrumented-${rt}`];
      const o = summaries[`${PREFIX}bench-oss-otel-${rt}`];

      const memVariants: [string, FunctionSummary | undefined][] = [["Baseline", b], ["Dash0", d], ["OSS OTel", o]];
      for (const [label, s] of memVariants) {
        if (s?.maxMemoryUsedMb) {
          const m = s.maxMemoryUsedMb;
          lines.push(
            `| ${displayName} | ${label} | ${m.min} | ${m.avg} | ${m.max} | ${m.values.length} |`
          );
        }
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
  const allFunctions = [
    ...ALL_RUNTIMES.flatMap((rt) => [
      `${PREFIX}bench-baseline-${rt}`,
      `${PREFIX}bench-instrumented-${rt}`,
      `${PREFIX}bench-oss-otel-${rt}`,
    ]),
    `${PREFIX}bench-extension-only-${EXTENSION_ONLY_FUNCTION}`,
  ];

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
