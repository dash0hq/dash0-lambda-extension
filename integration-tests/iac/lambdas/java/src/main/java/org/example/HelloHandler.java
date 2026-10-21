package org.example;

import com.amazonaws.services.lambda.runtime.Context;
import com.amazonaws.services.lambda.runtime.RequestHandler;

import java.net.URI;
import java.net.http.HttpClient;
import java.net.http.HttpRequest;
import java.net.http.HttpResponse;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;

public class HelloHandler implements RequestHandler<Object, String> {

    @Override
    public String handleRequest(Object input, Context context) {
        context.getLogger().log("Input received: " + String.valueOf(input));

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

        // Only the success scenario sets POST_MASKED_FIELD. The stack sets DASH0_MASK_RULES so that
        // the Java distro masks masked_field in the captured request body and the x-masked-field
        // header (OTEL_INSTRUMENTATION_HTTP_CLIENT_CAPTURE_REQUEST_HEADERS).
        if ("true".equals(System.getenv("POST_MASKED_FIELD"))) {
            postMaskedField(context);
        }

        return "Hello World from Java Lambda!";
    }

    private static void postMaskedField(Context context) {
        String body = "{\"title\":\"foo\",\"body\":\"bar\",\"userId\":1,\"masked_field\":\"this should not be seen!\"}";
        HttpRequest request = HttpRequest.newBuilder(URI.create("https://jsonplaceholder.typicode.com/posts"))
                .header("Content-Type", "application/json")
                .header("x-masked-field", "this should not be seen!")
                .POST(HttpRequest.BodyPublishers.ofString(body))
                .build();
        try {
            HttpResponse<String> response = HttpClient.newHttpClient().send(request, HttpResponse.BodyHandlers.ofString());
            context.getLogger().log("response.statusCode: " + response.statusCode());
        } catch (Exception e) {
            context.getLogger().log("HTTP request failed: " + e);
            if (e instanceof InterruptedException) {
                Thread.currentThread().interrupt();
            }
        }
    }
}
