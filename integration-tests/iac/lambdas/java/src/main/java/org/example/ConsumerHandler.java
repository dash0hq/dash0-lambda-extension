package org.example;

import com.amazonaws.services.lambda.runtime.Context;
import com.amazonaws.services.lambda.runtime.RequestHandler;
import com.amazonaws.services.lambda.runtime.events.SQSEvent;

import java.util.LinkedHashMap;
import java.util.Map;

public class ConsumerHandler implements RequestHandler<SQSEvent, Map<String, Object>> {

    @Override
    public Map<String, Object> handleRequest(SQSEvent event, Context context) {
        context.getLogger().log("Received " + event.getRecords().size() + " record(s)");

        for (int i = 0; i < event.getRecords().size(); i++) {
            SQSEvent.SQSMessage record = event.getRecords().get(i);
            context.getLogger().log("Record " + i + ": " + record.getBody());
        }

        Map<String, Object> result = new LinkedHashMap<>();
        result.put("statusCode", 200);
        result.put("body", "{\"records_processed\":" + event.getRecords().size() + "}");
        return result;
    }
}
