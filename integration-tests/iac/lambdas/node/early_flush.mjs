import https from 'https';

// Runs long enough for the extension to flush while the invocation is still
// running (the stack sets DASH0_FLUSH_INTERVAL=1000), and logs before and
// after a child span, so logs are sent before the handler span exists.
export async function handler(event) {
    console.log("early-flush: before child span");

    const statusCode = await new Promise((resolve, reject) => {
        https.get('https://jsonplaceholder.typicode.com/posts/1', (res) => {
            res.resume();
            res.on('end', () => resolve(res.statusCode));
        }).on('error', reject);
    });

    await new Promise((resolve) => setTimeout(resolve, 2500));
    console.log(`early-flush: after child span ${statusCode}`);
    await new Promise((resolve) => setTimeout(resolve, 1500));
    console.log("early-flush: done");

    return { statusCode: 200 };
}
