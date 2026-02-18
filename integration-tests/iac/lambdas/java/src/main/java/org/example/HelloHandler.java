package org.example;

import com.amazonaws.services.lambda.runtime.Context;
import com.amazonaws.services.lambda.runtime.RequestHandler;

import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;

public class HelloHandler implements RequestHandler<Object, String> {

    @Override
    public String handleRequest(Object input, Context context) {
        context.getLogger().log("Received: " + String.valueOf(input));

        String sleepEnv = System.getenv("SLEEP_DURATION_MS");
        long sleepMs = 1000L;
        if (sleepEnv != null && !sleepEnv.isEmpty()) {
            try {
                sleepMs = Long.parseLong(sleepEnv);
            } catch (NumberFormatException e) {
                context.getLogger().log("Invalid SLEEP_DURATION_MS: " + sleepEnv);
            }
        }

        try {
            Thread.sleep(sleepMs);
        } catch (InterruptedException e) {
            context.getLogger().log("Sleep interrupted: " + e.getMessage());
            Thread.currentThread().interrupt();
        }

        if (input instanceof LinkedHashMap) {
            LinkedHashMap<?, ?> map = (LinkedHashMap<?, ?>) input;
            Object param = map.get("parameter1");
            if ("throw".equals(param)) {
                throw new RuntimeException("Intentional exception triggered by input 'throw'");
            }
            if ("outofmemory".equals(param)) {
                context.getLogger().log("Triggering OutOfMemoryError...");
                List<byte[]> list = new ArrayList<>();
                while (true) {
                    list.add(new byte[1048576]);
                }
            }
        }

        return "Hello World from Java Lambda!";
    }
}
