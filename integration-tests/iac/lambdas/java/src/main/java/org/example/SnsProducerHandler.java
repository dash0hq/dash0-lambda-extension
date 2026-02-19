package org.example;

import com.amazonaws.services.lambda.runtime.Context;
import com.amazonaws.services.lambda.runtime.RequestHandler;

import software.amazon.awssdk.services.sns.SnsClient;
import software.amazon.awssdk.services.sns.model.PublishRequest;
import software.amazon.awssdk.services.sns.model.PublishResponse;

import java.util.LinkedHashMap;
import java.util.Map;

public class SnsProducerHandler implements RequestHandler<Object, Map<String, Object>> {

    private final SnsClient snsClient = SnsClient.create();

    @Override
    public Map<String, Object> handleRequest(Object input, Context context) {
        String topicArn = System.getenv("TOPIC_ARN");

        String message = "{\"message\":\"Hello from SNS producer!\",\"request_id\":\"" + context.getAwsRequestId() + "\"}";

        PublishResponse response = snsClient.publish(PublishRequest.builder()
                .topicArn(topicArn)
                .message(message)
                .subject("Test Message")
                .build());

        context.getLogger().log("Published message to SNS: " + response.messageId());

        Map<String, Object> result = new LinkedHashMap<>();
        result.put("statusCode", 200);
        result.put("body", "{\"message_id\":\"" + response.messageId() + "\"}");
        return result;
    }
}
