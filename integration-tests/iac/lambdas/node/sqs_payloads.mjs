import { SQSClient, SendMessageCommand, SendMessageBatchCommand, ReceiveMessageCommand } from '@aws-sdk/client-sqs';

const sqs = new SQSClient();

export async function handler() {
    const QueueUrl = process.env.QUEUE_URL;
    await sqs.send(new SendMessageCommand({
        QueueUrl,
        MessageBody: 'payload-single-secret',
    }));
    const response = await sqs.send(new SendMessageBatchCommand({
        QueueUrl,
        Entries: [
            { Id: 'order-a', MessageBody: 'payload-batch-secret-a' },
            { Id: 'order-b', MessageBody: 'payload-batch-secret-b' },
        ],
    }));
    if (response.Failed?.length) {
        throw new Error(`SQS batch send failed: ${JSON.stringify(response.Failed)}`);
    }
    const received = await sqs.send(new ReceiveMessageCommand({
        QueueUrl,
        MaxNumberOfMessages: 1,
        WaitTimeSeconds: 2,
    }));
    if (!received.Messages?.length) {
        throw new Error('SQS payload fixture received no messages');
    }
    console.log('SQS payload sends and receive completed');
    return { statusCode: 200 };
}
