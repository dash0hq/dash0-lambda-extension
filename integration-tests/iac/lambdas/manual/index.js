'use strict';

// Set Lambda env vars - force _HANDLER to point to actual handler module
// so instrumentation patches the right module
process.env.LAMBDA_TASK_ROOT = process.env.LAMBDA_TASK_ROOT || __dirname;
process.env._HANDLER = 'handler.hello';

// Initialize tracing BEFORE loading the handler
require('./tracing');

// Export the handler - instrumentation will wrap it automatically
module.exports = require('./handler');
