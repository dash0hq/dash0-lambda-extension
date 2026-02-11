import { SQSClient, SendMessageCommand } from '@aws-sdk/client-sqs';

const sqs = new SQSClient();

export async function handler(event, context) {
    const queueUrl = process.env.QUEUE_URL;

    const message = {
        message: 'Hello from SQS producer!',
        request_id: context.awsRequestId,
    };

    const response = await sqs.send(new SendMessageCommand({
        QueueUrl: queueUrl,
        MessageBody: JSON.stringify(message),
    }));

    console.log(`Sent message to SQS: ${response.MessageId}`);

    return {
        statusCode: 200,
        body: JSON.stringify({
            message_id: response.MessageId,
        }),
    };
}
