import { LambdaClient, InvokeCommand } from '@aws-sdk/client-lambda';

const lambdaClient = new LambdaClient();

export async function handler(event, context) {
    const targetFunctionName = process.env.TARGET_FUNCTION_NAME;

    const payload = {
        message: 'Hello from Lambda invoker!',
        request_id: context.awsRequestId,
    };

    const response = await lambdaClient.send(new InvokeCommand({
        FunctionName: targetFunctionName,
        InvocationType: 'RequestResponse',
        Payload: JSON.stringify(payload),
    }));

    const responsePayload = JSON.parse(Buffer.from(response.Payload).toString());
    console.log(`Invoked Lambda ${targetFunctionName}, status: ${response.StatusCode}`);

    return {
        statusCode: 200,
        body: JSON.stringify({
            invoked_function: targetFunctionName,
            status_code: response.StatusCode,
            response: responsePayload,
        }),
    };
}
