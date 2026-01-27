'use strict';

// Use the same entry point as Lambda
const { hello } = require('./index');
const { provider } = require('./tracing');

const event = {
  httpMethod: 'GET',
  path: '/hello',
  headers: {},
  queryStringParameters: null,
  body: null,
};

const context = {
  functionName: 'check-node-metrics',
  functionVersion: '$LATEST',
  invokedFunctionArn: 'arn:aws:lambda:us-west-2:123456789012:function:check-node-metrics',
  memoryLimitInMB: '128',
  awsRequestId: 'local-test-' + Date.now(),
  logGroupName: '/aws/lambda/check-node-metrics',
  logStreamName: '2024/01/01/[$LATEST]abc123',
  getRemainingTimeInMillis: () => 30000,
};

(async () => {
  try {
    const response = await hello(event, context);
    console.log('Response:', JSON.stringify(response, null, 2));
  } catch (err) {
    console.error('Error:', err);
    process.exit(1);
  } finally {
    // Ensure all spans are flushed before exit
    await provider.forceFlush();
    await provider.shutdown();
  }
})();
