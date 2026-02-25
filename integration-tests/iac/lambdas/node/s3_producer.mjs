import { S3Client, PutObjectCommand } from '@aws-sdk/client-s3';

const s3 = new S3Client();

export async function handler(event, context) {
    const bucketName = process.env.BUCKET_NAME;

    const body = {
        message: 'Hello from S3 producer!',
        requestId: context.awsRequestId,
    };

    const key = `test-events/${context.awsRequestId}.json`;

    const response = await s3.send(new PutObjectCommand({
        Bucket: bucketName,
        Key: key,
        Body: JSON.stringify(body),
        ContentType: 'application/json',
    }));

    console.log(`Put object to S3: ${key}, ETag: ${response.ETag}`);

    return {
        statusCode: 200,
        body: JSON.stringify({
            key,
            etag: response.ETag,
        }),
    };
}
