import { EventBridgeClient, PutEventsCommand } from '@aws-sdk/client-eventbridge';

const events = new EventBridgeClient();

export async function handler(event, context) {
    const eventBusName = process.env.EVENT_BUS_NAME;

    const detail = {
        message: 'Hello from EventBridge producer!',
        requestId: context.awsRequestId,
    };

    const response = await events.send(new PutEventsCommand({
        Entries: [
            {
                Source: 'tracing-tests.producer',
                DetailType: 'TestMessage',
                Detail: JSON.stringify(detail),
                EventBusName: eventBusName,
            },
        ],
    }));

    console.log(`Put event to EventBridge: ${JSON.stringify(response)}`);

    return {
        statusCode: 200,
        body: JSON.stringify({
            failed_entry_count: response.FailedEntryCount,
        }),
    };
}
