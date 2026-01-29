exports.handler = async (event, context) => {
    console.log(`event payload: ${JSON.stringify(event)}`);
    return {
        statusCode: 200,
        body: '{"message":"Success"}'
    };
};
