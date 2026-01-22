import path from "path";
import { fileURLToPath } from "url";
import webpack from "webpack";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

export default {
  mode: "production",
  target: "node18",
  entry: "./init.mjs",
  output: {
    path: path.resolve(__dirname, "dist"),
    filename: "init.mjs",
    library: {
      type: "module",
    },
    chunkFormat: "module",
  },
  experiments: {
    outputModule: true,
  },
  optimization: {
    minimize: true,
    moduleIds: "named",
    chunkIds: "named",
  },
  resolve: {
    extensions: [".mjs", ".js", ".json"],
  },
  externalsType: "module",
  externals: [
    // Node.js built-ins - use node: prefix for ESM
    ({ request }, callback) => {
      const nodeBuiltins = [
        "fs", "path", "http", "https", "net", "tls", "dns", "os", "stream",
        "zlib", "crypto", "events", "util", "buffer", "url", "querystring",
        "child_process", "cluster", "dgram", "readline", "repl", "tty", "vm",
        "worker_threads", "perf_hooks", "async_hooks", "inspector", "module",
        "assert", "diagnostics_channel", "string_decoder", "timers", "console",
        "process", "v8", "trace_events", "constants"
      ];

      if (nodeBuiltins.includes(request)) {
        return callback(null, `node:${request}`);
      }

      // External npm packages that need require()
      const requireExternals = [
        "require-in-the-middle",
        "import-in-the-middle",
        "module-details-from-path",
      ];

      if (requireExternals.includes(request)) {
        // Use commonjs for these since they're CJS modules
        return callback(null, `commonjs ${request}`);
      }

      // User libraries - keep as external
      const userLibs = [
        "express", "fastify", "koa", "hapi", "pg", "mysql", "mysql2",
        "mongodb", "mongoose", "redis", "ioredis", "amqplib", "kafkajs",
        "aws-sdk", "graphql", "@grpc/grpc-js", "winston", "bunyan", "pino",
        "@prisma/client"
      ];

      if (userLibs.includes(request) || request.startsWith("@aws-sdk/")) {
        return callback(null, `commonjs ${request}`);
      }

      callback();
    },
  ],
  plugins: [
    new webpack.BannerPlugin({
      banner: `
import { createRequire } from 'module';
import { fileURLToPath } from 'url';
import { dirname } from 'path';
const require = createRequire(import.meta.url);
const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);
`,
      raw: true,
    }),
  ],
};
