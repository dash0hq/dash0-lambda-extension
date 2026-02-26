import { STSClient, GetCallerIdentityCommand } from "@aws-sdk/client-sts";

const stsClient = new STSClient({});

export async function handler(event: any) {
    console.log("Handler invoked with event:", event);
    const identity = await stsClient.send(new GetCallerIdentityCommand({}));
    console.log("Caller identity:", identity.Arn);
    await new Promise((resolve) => setTimeout(resolve, 2000));
    return {
        statusCode: 200,
        body: JSON.stringify({ message: "Success" }),
    };
}
