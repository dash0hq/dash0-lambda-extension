

export async function handler(event) {
    console.log("Handler invoked with event:", event);
    await new Promise((resolve) => setTimeout(resolve, 20000));
    return {
        statusCode: 200,
        body: JSON.stringify({ message: "Success" }),
    };
}