package org.example;

import com.amazonaws.services.lambda.runtime.Context;
import com.amazonaws.services.lambda.runtime.RequestHandler;
import com.amazonaws.services.lambda.runtime.events.SNSEvent;

import java.util.LinkedHashMap;
import java.util.Map;

public class SnsConsumerHandler implements RequestHandler<SNSEvent, Map<String, Object>> {

    @Override
    public Map<String, Object> handleRequest(SNSEvent event, Context context) {
        context.getLogger().log("Received event: " + event);
        context.getLogger().log("Received " + event.getRecords().size() + " record(s)");

        for (int i = 0; i < event.getRecords().size(); i++) {
            SNSEvent.SNSRecord record = event.getRecords().get(i);
            context.getLogger().log("Record " + i + ": " + record.getSNS().getMessage());
        }

        Map<String, Object> result = new LinkedHashMap<>();
        result.put("statusCode", 200);
        result.put("body", "{\"records_processed\":" + event.getRecords().size() + "}");
        return result;
    }
}
