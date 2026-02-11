import { SNSClient, PublishCommand } from '@aws-sdk/client-sns';

const sns = new SNSClient();

export async function handler(event, context) {
    const topicArn = process.env.TOPIC_ARN;

    const message = {
        message: 'Hello from SNS producer!',
        request_id: context.awsRequestId,
    };

    const response = await sns.send(new PublishCommand({
        TopicArn: topicArn,
        Message: JSON.stringify(message),
        Subject: 'Test Message',
    }));

    console.log(`Published message to SNS: ${response.MessageId}`);

    return {
        statusCode: 200,
        body: JSON.stringify({
            message_id: response.MessageId,
        }),
    };
}
