import https from 'https';

// Payloads the Node distro does not mask: the URL query (upstream redacts only a fixed list of
// signature parameters) and a form-encoded body (written as a quoted string). With
// DASH0_MASK_IN_EXTENSION=true the extension masks both with the default patterns.
const SECRET = 'XQZ-IT-SECRET';

export async function handler() {
    const body = `email=jane%40example.com&password=${SECRET}`;
    const status = await new Promise((resolve, reject) => {
        const req = https.request(`https://jsonplaceholder.typicode.com/posts?api_token=${SECRET}&page=1`, {
            method: 'POST',
            headers: {
                'Content-Type': 'application/x-www-form-urlencoded',
                'Content-Length': Buffer.byteLength(body),
            },
        }, (res) => {
            res.resume();
            res.on('end', () => resolve(res.statusCode));
        });
        req.on('error', reject);
        req.write(body);
        req.end();
    });
    return { statusCode: status };
}
