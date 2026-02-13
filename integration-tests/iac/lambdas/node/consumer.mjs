export async function handler(event, context) {
    console.log(`Received event: ${JSON.stringify(event)}`);

    // Check if this is an EventBridge event (has detail-type field)
    if (event['detail-type']) {
        console.log(`EventBridge event from source: ${event.source}`);
        console.log(`Detail type: ${event['detail-type']}`);
        console.log(`Detail: ${JSON.stringify(event.detail || {})}`);
        return {
            statusCode: 200,
            body: JSON.stringify({
                event_type: 'eventbridge',
                source: event.source,
                detail_type: event['detail-type'],
            }),
        };
    }

    // Handle Records-based events (SQS, SNS)
    const records = event.Records || [];
    console.log(`Processing ${records.length} record(s)`);

    for (let i = 0; i < records.length; i++) {
        console.log(`Record ${i}: ${JSON.stringify(records[i])}`);
    }

    return {
        statusCode: 200,
        body: JSON.stringify({
            records_processed: records.length,
        }),
    };
}
