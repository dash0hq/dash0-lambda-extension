'use strict';

// In-app OpenTelemetry SDK initialization, mirroring what Starling does: the
// application boots its own OTel SDK before anything else. When the Dash0
// auto-instrumentation is also loaded (via NODE_OPTIONS --import /opt/init.mjs),
// this is what triggers "@opentelemetry/api: Attempted duplicate registration".
if ((process.env.INIT_APP_OTEL || '').toLowerCase() === 'true') {
  require('./otel');
}

const express = require('express');
const https = require('https');

const app = express();
const port = parseInt(process.env.PORT || '8080', 10);

function httpsGet(url) {
  return new Promise((resolve, reject) => {
    https
      .get(url, (res) => {
        let body = '';
        res.on('data', (chunk) => (body += chunk));
        res.on('end', () => resolve({ statusCode: res.statusCode, body: body.trim() }));
      })
      .on('error', reject);
  });
}

function status() {
  return {
    functionName: process.env.AWS_LAMBDA_FUNCTION_NAME,
    scenario: process.env.SCENARIO || 'unknown',
    execWrapper: process.env.AWS_LAMBDA_EXEC_WRAPPER,
    runtimeApi: process.env.AWS_LAMBDA_RUNTIME_API,
    nodeOptions: process.env.NODE_OPTIONS || '',
    inAppOtelSdk: (process.env.INIT_APP_OTEL || '').toLowerCase() === 'true',
    otlpEndpoint:
      process.env.OTEL_EXPORTER_OTLP_TRACES_ENDPOINT ||
      process.env.OTEL_EXPORTER_OTLP_ENDPOINT ||
      '(exporter default)',
  };
}

app.get('/', (req, res) => {
  console.log(`[app] GET / on ${process.env.SCENARIO || 'unknown'}`);
  res.json({ message: 'hello from the SUP-1312 playground', ...status() });
});

// Makes an outbound HTTPS call so instrumentation (Dash0 distro or in-app SDK)
// can produce an HTTP client span.
app.get('/downstream', async (req, res) => {
  console.log('[app] GET /downstream - calling checkip.amazonaws.com');
  try {
    const upstream = await httpsGet('https://checkip.amazonaws.com/');
    res.json({ upstreamStatus: upstream.statusCode, egressIp: upstream.body, ...status() });
  } catch (err) {
    console.error('[app] downstream call failed', err);
    res.status(502).json({ error: String(err), ...status() });
  }
});

app.get('/error', (req, res) => {
  console.error('[app] GET /error - returning 500 on purpose');
  res.status(500).json({ error: 'intentional error for testing', ...status() });
});

app.listen(port, () => {
  console.log(`[app] express listening on port ${port}`, JSON.stringify(status()));
});
