import { KinesisClient, PutRecordCommand } from '@aws-sdk/client-kinesis';

const kinesis = new KinesisClient();

export async function handler(event, context) {
    const streamName = process.env.STREAM_NAME;

    const message = {
        message: 'Hello from Kinesis producer!',
        request_id: context.awsRequestId,
    };

    const response = await kinesis.send(new PutRecordCommand({
        StreamName: streamName,
        Data: Buffer.from(JSON.stringify(message)),
        PartitionKey: context.awsRequestId,
    }));

    console.log(`Put record to Kinesis: ${response.SequenceNumber}`);

    return {
        statusCode: 200,
        body: JSON.stringify({
            sequence_number: response.SequenceNumber,
        }),
    };
}
