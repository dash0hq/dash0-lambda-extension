import https from 'https';

export async function handler(event) {
    console.log("Handler invoked with event:", event);

    const postData = JSON.stringify({
        title: 'foo',
        body: 'bar',
        userId: 1,
        // Masked by the Node distro via DASH0_MASK_RULES, set on the function by the stack.
        masked_field: 'this should not be seen!',
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
    console.log(JSON.stringify({ jsonLog: true, value: "parsed" }));
    // Raw stdout skips the Node runtime's level prefix, so only the JSON `level` field carries severity.
    process.stdout.write(JSON.stringify({ level: "error", jsonSeverity: "inferred" }) + "\n");

    return {
        statusCode: 200,
        body: JSON.stringify({ message: "Success" }),
    };
}
