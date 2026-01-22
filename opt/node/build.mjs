import * as esbuild from "esbuild";
import { readFileSync } from "fs";

const pkg = JSON.parse(readFileSync("./package.json", "utf8"));

async function build() {
  const startTime = Date.now();

  try {
    const result = await esbuild.build({
      entryPoints: ["./init.mjs"],
      bundle: true,
      platform: "node",
      target: "node18",
      format: "esm",
      outfile: "./dist/init.mjs",
      minify: true, // Minify for production
      sourcemap: true,
      metafile: true,

      // These need to stay external - they're Node.js built-ins or have native components
      external: [
        // Node.js built-ins
        "fs",
        "path",
        "http",
        "https",
        "net",
        "tls",
        "dns",
        "os",
        "stream",
        "zlib",
        "crypto",
        "events",
        "util",
        "buffer",
        "url",
        "querystring",
        "child_process",
        "cluster",
        "dgram",
        "readline",
        "repl",
        "tty",
        "vm",
        "worker_threads",
        "perf_hooks",
        "async_hooks",
        "inspector",
        "module",
        "assert",
        "diagnostics_channel",
        "string_decoder",
        "timers",
        "console",
        "process",
        "v8",
        "trace_events",
        "constants",

        // These use require-in-the-middle hooks that won't work if bundled
        // They need to intercept the user's require() calls
        "require-in-the-middle",
        "import-in-the-middle",
        "module-details-from-path",

        // Libraries that users will import (instrumentations need to hook these)
        // These should NOT be bundled - the instrumentation hooks need to intercept them
        "express",
        "fastify",
        "koa",
        "hapi",
        "pg",
        "mysql",
        "mysql2",
        "mongodb",
        "mongoose",
        "redis",
        "ioredis",
        "amqplib",
        "kafkajs",
        "aws-sdk",
        "@aws-sdk/*",
        "graphql",
        "@grpc/grpc-js",
        "winston",
        "bunyan",
        "pino",
        "@prisma/client",
      ],

      // Banner to create require() for ESM compatibility with dynamic requires
      banner: {
        js: `/* Bundled with esbuild */
import { createRequire } from 'module';
import { fileURLToPath } from 'url';
import { dirname } from 'path';
const require = createRequire(import.meta.url);
const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);
`,
      },
    });

    // Analyze the bundle
    const text = await esbuild.analyzeMetafile(result.metafile, {
      verbose: false,
    });

    console.log("Build completed in", Date.now() - startTime, "ms");
    console.log("\nBundle analysis:");
    console.log(text);

    // Output size info
    const outputs = result.metafile.outputs;
    for (const [file, info] of Object.entries(outputs)) {
      if (!file.endsWith(".map")) {
        console.log(`\n${file}: ${(info.bytes / 1024).toFixed(1)} KB`);
      }
    }
  } catch (error) {
    console.error("Build failed:", error);
    process.exit(1);
  }
}

build();
