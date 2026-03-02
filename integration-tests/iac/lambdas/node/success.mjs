export async function handler(event) {
    console.log("Handler invoked with event:", event);

    const response = await fetch('https://jsonplaceholder.typicode.com/posts', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
            title: 'foo',
            body: 'bar',
            userId: 1,
        }),
    });

    console.log(`response.statusCode: ${response.status}`);
    console.warn("let's parse this as a warning");

    return {
        statusCode: 200,
        body: JSON.stringify({ message: "Success" }),
    };
}
