'use strict';

const { meter } = require('./tracing');

// Create a counter metric
const invocationCounter = meter.createCounter('lambda.invocations', {
  description: 'Number of Lambda invocations',
});

module.exports.hello = async (event, context) => {
  // Record a metric datapoint
  invocationCounter.add(1, {
    'function.name': context?.functionName || 'unknown',
  });

  return {
    statusCode: 200,
    body: JSON.stringify({ message: "Success" }),
  };
};
