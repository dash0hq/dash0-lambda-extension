import https from 'https';

export async function handler(event, context) {
    const apiUrl = process.env.API_URL;

    const payload = JSON.stringify({
        message: 'Hello from API Gateway producer!',
        requestId: context.awsRequestId,
    });

    const response = await new Promise((resolve, reject) => {
        const url = new URL(apiUrl);
        const options = {
            hostname: url.hostname,
            path: url.pathname,
            method: 'POST',
            headers: {
                'Content-Type': 'application/json',
                'Content-Length': Buffer.byteLength(payload),
            },
        };

        const req = https.request(options, (res) => {
            let data = '';
            res.on('data', (chunk) => { data += chunk; });
            res.on('end', () => {
                resolve({ statusCode: res.statusCode, body: data });
            });
        });

        req.on('error', reject);
        req.write(payload);
        req.end();
    });

    console.log(`API Gateway response: ${response.statusCode} ${response.body}`);

    return {
        statusCode: 200,
        body: JSON.stringify({
            api_status_code: response.statusCode,
            api_response: response.body,
        }),
    };
}
