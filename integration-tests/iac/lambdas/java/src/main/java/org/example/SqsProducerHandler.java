package org.example;

import com.amazonaws.services.lambda.runtime.Context;
import com.amazonaws.services.lambda.runtime.RequestHandler;

import software.amazon.awssdk.services.sqs.SqsClient;
import software.amazon.awssdk.services.sqs.model.SendMessageRequest;
import software.amazon.awssdk.services.sqs.model.SendMessageResponse;

import java.util.LinkedHashMap;
import java.util.Map;

public class SqsProducerHandler implements RequestHandler<Object, Map<String, Object>> {

    private final SqsClient sqsClient = SqsClient.create();

    @Override
    public Map<String, Object> handleRequest(Object input, Context context) {
        String queueUrl = System.getenv("QUEUE_URL");

        String messageBody = "{\"message\":\"Hello from SQS producer!\",\"request_id\":\"" + context.getAwsRequestId() + "\"}";

        SendMessageResponse response = sqsClient.sendMessage(SendMessageRequest.builder()
                .queueUrl(queueUrl)
                .messageBody(messageBody)
                .build());

        context.getLogger().log("Sent message to SQS: " + response.messageId());

        Map<String, Object> result = new LinkedHashMap<>();
        result.put("statusCode", 200);
        result.put("body", "{\"message_id\":\"" + response.messageId() + "\"}");
        return result;
    }
}
