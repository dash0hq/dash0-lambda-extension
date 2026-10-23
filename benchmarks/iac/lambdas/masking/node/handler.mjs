import https from "node:https";

// Same body in every runtime: ~3.9 KB of compact JSON with three secret-looking keys.
function body(n) {
  const items = [];
  for (let i = 0; i < 41; i++) {
    items.push({ sku: `SKU-${String(i).padStart(4, "0")}`, name: `Item number ${i} with a short description`, qty: (i % 5) + 1, priceCents: i * 100 + 99 });
  }
  return JSON.stringify({
    orderId: n,
    customer: { name: "Jane Doe", email: "jane@example.com", password: `pw-SECRETVALUE-${n}` },
    auth: { api_key: `key-SECRETVALUE-${n}`, access_token: `tok-SECRETVALUE-${n}` },
    items,
  });
}

function post(url, payload) {
  return new Promise((resolve, reject) => {
    const req = https.request(url, { method: "POST", headers: { "content-type": "application/json" } }, (res) => {
      const chunks = [];
      res.on("data", (c) => chunks.push(c));
      res.on("end", () => resolve(Buffer.concat(chunks).toString()));
    });
    req.on("error", reject);
    req.end(payload);
  });
}

export async function handler(event) {
  let echoed = 0;
  for (let n = 0; n < 5; n++) {
    echoed += (await post(process.env.ECHO_URL, body(n))).length;
  }
  return { statusCode: 200, body: JSON.stringify({ orderId: event.orderId ?? null, echoed }) };
}
