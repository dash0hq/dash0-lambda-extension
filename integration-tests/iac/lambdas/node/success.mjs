import https from 'https';

export async function handler(event) {
    console.log("Handler invoked with event:", event);

    const postData = JSON.stringify({
        title: 'foo',
        body: 'bar',
        userId: 1,
    });

    const response = await new Promise((resolve, reject) => {
        const req = https.request('https://jsonplaceholder.typicode.com/posts', {
            method: 'POST',
            headers: {
                'Content-Type': 'application/json',
                'Content-Length': Buffer.byteLength(postData),
            },
        }, (res) => {
            let data = '';
            res.on('data', (chunk) => { data += chunk; });
            res.on('end', () => { resolve({ statusCode: res.statusCode, body: data }); });
        });
        req.on('error', reject);
        req.write(postData);
        req.end();
    });

    console.log(`response.statusCode: ${response.statusCode}`);
    console.warn("let's parse this as a warning");

    return {
        statusCode: 200,
        body: JSON.stringify({ message: "Success" }),
    };
}
