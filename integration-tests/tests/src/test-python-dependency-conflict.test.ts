import fetch from 'node-fetch';
import { setTimeout as delay } from 'node:timers/promises';
import { describe, expect, it } from 'vitest';
import { DASH0_ENDPOINT, DASH0_TOKEN, MAX_ATTEMPTS, RETRY_DELAY_MS } from "./config";
import { invokeFunction, RESOURCE_PREFIX } from "./utils";


const verifyDependencyConflict = async (functionName: string) => {
    await invokeFunction(functionName, true, false);

    const now = Date.now();
    for (let attempt = 1; attempt <= MAX_ATTEMPTS; attempt++) {
        await delay(RETRY_DELAY_MS);
        console.log(`Attempt ${attempt} to fetch logs for function ${functionName}`);
        try {
            const logsResponse = await fetch(DASH0_ENDPOINT + 'logs', {
                method: 'POST',
                headers: {
                    accept: 'application/json',
                    authorization: `Bearer ${DASH0_TOKEN}`,
                    'content-type': 'application/json',
                },
                body: JSON.stringify({
                    filter: [
                        {
                            operator: 'is',
                            key: 'service.name',
                            value: functionName,
                        },
                    ],
                    timeRange: {
                        from: new Date(now - 5 * 60_000).toISOString(),
                        to: new Date(now + 5 * 60_000).toISOString(),
                    },
                    sampling: { mode: 'adaptive' },
                }),
            });

            const logsPayload = await logsResponse.json() as any;
            expect(logsPayload?.resourceLogs.length).toBeGreaterThanOrEqual(1);
            const allLogRecords = logsPayload.resourceLogs.flatMap(
                (rl: any) => rl.scopeLogs.flatMap((sl: any) => sl.logRecords)
            );
            const foundConflictLog = allLogRecords.some(
                (record: any) => record.body.stringValue.includes('Skipping instrumentation due to dependency conflict: opentelemetry-proto requires protobuf')
            );
            expect(foundConflictLog).toBeTruthy();
            break;
        } catch (error) {
            console.error(`Error fetching logs on attempt ${attempt}:`, error);
            if (attempt === MAX_ATTEMPTS) {
                throw error;
            }
        }
    }
};

describe.concurrent('Python dependency conflict', { retry: 1 }, () => {
    const runtimes = ['python3-10', 'python3-11', 'python3-12', 'python3-13'];

    for (const runtime of runtimes) {
        const functionName = `${RESOURCE_PREFIX}dependency-conflict-${runtime}`;
        it(
            `verifies dependency conflict detection for ${functionName}`,
            async () => {
                console.log(`Starting test for ${functionName}`, new Date().toISOString());
                await verifyDependencyConflict(functionName);
            },
            120_000,
        );
    }
});
