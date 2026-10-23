import { gunzipSync } from "node:zlib";

// One Function URL serves as the echo target (any path) and as the OTLP endpoint (/v1/*).
// For OTLP it only counts secret values and masks per sending function, so masking can be verified.
export async function handler(event) {
  const path = event.rawPath ?? "/";
  let buf = Buffer.from(event.body ?? "", event.isBase64Encoded ? "base64" : "utf8");
  if (!path.startsWith("/v1/")) {
    return { statusCode: 200, headers: { "content-type": "application/json" }, body: buf.toString() };
  }
  const enc = event.headers?.["content-encoding"] ?? "";
  if (enc.includes("gzip")) buf = gunzipSync(buf);
  const text = buf.toString("latin1");
  const fn = text.match(/cmask-bench-[a-z0-9-]+/)?.[0] ?? "unknown";
  const count = (s) => text.split(s).length - 1;
  console.log(JSON.stringify({ otlp: path, fn, bytes: buf.length, leaks: count("SECRETVALUE"), masked: count("****") }));
  return { statusCode: 200, headers: { "content-type": "application/x-protobuf" }, body: "" };
}
