import https from 'https';

// Payloads the Node distro does not mask: the URL query (upstream redacts only a fixed list of
// signature parameters) and a form-encoded body (written as a quoted string). Plus a JSON body and
// an Authorization header, which the distro masks unless DASH0_MASK_IN_EXTENSION=true hands
// masking to the extension. The extension masks all of them with the default patterns.
const SECRET = 'XQZ-IT-SECRET';

const post = (url, contentType, body, headers = {}) =>
    new Promise((resolve, reject) => {
        const req = https.request(url, {
            method: 'POST',
            headers: {
                'Content-Type': contentType,
                'Content-Length': Buffer.byteLength(body),
                ...headers,
            },
        }, (res) => {
            res.resume();
            res.on('end', () => resolve(res.statusCode));
        });
        req.on('error', reject);
        req.write(body);
        req.end();
    });

export async function handler() {
    const status = await post(
        `https://jsonplaceholder.typicode.com/posts?api_token=${SECRET}&page=1`,
        'application/x-www-form-urlencoded',
        `email=jane%40example.com&password=${SECRET}`,
    );
    await post(
        'https://jsonplaceholder.typicode.com/posts',
        'application/json',
        JSON.stringify({ email: 'jane@example.com', password: SECRET }),
        { Authorization: `Bearer ${SECRET}` },
    );
    return { statusCode: status };
}
