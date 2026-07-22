'use strict';

// Plain Lambda handler for the Dash0-only baseline (no Web Adapter). Invoked
// through a Function URL, so it answers in the Function URL response format.

const https = require('https');

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

exports.handler = async (event) => {
  const path = event.rawPath || '/';
  console.log(`[handler] invoked with path ${path}`);

  let downstream = null;
  if (path === '/downstream') {
    const upstream = await httpsGet('https://checkip.amazonaws.com/');
    downstream = { upstreamStatus: upstream.statusCode, egressIp: upstream.body };
  }
  if (path === '/error') {
    console.error('[handler] returning 500 on purpose');
    return {
      statusCode: 500,
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ error: 'intentional error for testing' }),
    };
  }

  return {
    statusCode: 200,
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({
      message: 'hello from the SUP-1312 playground (plain handler)',
      functionName: process.env.AWS_LAMBDA_FUNCTION_NAME,
      scenario: process.env.SCENARIO || 'unknown',
      execWrapper: process.env.AWS_LAMBDA_EXEC_WRAPPER,
      runtimeApi: process.env.AWS_LAMBDA_RUNTIME_API,
      downstream,
    }),
  };
};
