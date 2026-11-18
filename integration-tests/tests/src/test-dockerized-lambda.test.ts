import { describe, it } from 'vitest';
import { RESOURCE_PREFIX, verifyDockerizedInvocation } from "./utils";
import {TEST_TIMEOUT_MS} from "./config";

describe.concurrent('Dockerized Lambda invocation', () => {
    const runtimes = ['python', 'node', 'java'] as const;
    const architectures = ['x86_64', 'arm64'];

    for (const runtime of runtimes) {
        for (const architecture of architectures) {
            const functionName = `${RESOURCE_PREFIX}dockerized-${runtime}-${architecture}`;
            it(functionName, async () => {
                await verifyDockerizedInvocation(functionName, runtime);
            }, TEST_TIMEOUT_MS);
        }
    }
});
