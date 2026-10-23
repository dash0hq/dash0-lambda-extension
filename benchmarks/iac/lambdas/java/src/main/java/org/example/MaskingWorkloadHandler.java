package org.example;

import com.amazonaws.services.lambda.runtime.Context;
import com.amazonaws.services.lambda.runtime.RequestHandler;
import java.net.URI;
import java.net.http.HttpClient;
import java.net.http.HttpRequest;
import java.net.http.HttpResponse;
import java.util.Map;

public class MaskingWorkloadHandler implements RequestHandler<Map<String, Object>, Map<String, Object>> {

    private static final HttpClient CLIENT = HttpClient.newHttpClient();

    // Same body in every runtime: ~3.9 KB of compact JSON with three secret-looking keys.
    static String body(int n) {
        StringBuilder items = new StringBuilder();
        for (int i = 0; i < 41; i++) {
            if (i > 0) items.append(',');
            items.append(String.format(
                "{\"sku\":\"SKU-%04d\",\"name\":\"Item number %d with a short description\",\"qty\":%d,\"priceCents\":%d}",
                i, i, i % 5 + 1, i * 100 + 99));
        }
        return "{\"orderId\":" + n
            + ",\"customer\":{\"name\":\"Jane Doe\",\"email\":\"jane@example.com\",\"password\":\"pw-SECRETVALUE-" + n + "\"}"
            + ",\"auth\":{\"api_key\":\"key-SECRETVALUE-" + n + "\",\"access_token\":\"tok-SECRETVALUE-" + n + "\"}"
            + ",\"items\":[" + items + "]}";
    }

    @Override
    public Map<String, Object> handleRequest(Map<String, Object> event, Context context) {
        URI echo = URI.create(System.getenv("ECHO_URL"));
        int echoed = 0;
        try {
            for (int n = 0; n < 5; n++) {
                HttpRequest req = HttpRequest.newBuilder(echo)
                    .header("content-type", "application/json")
                    .POST(HttpRequest.BodyPublishers.ofString(body(n)))
                    .build();
                echoed += CLIENT.send(req, HttpResponse.BodyHandlers.ofString()).body().length();
            }
        } catch (Exception e) {
            throw new RuntimeException(e);
        }
        return Map.of("statusCode", 200, "body", "{\"orderId\":" + event.get("orderId") + ",\"echoed\":" + echoed + "}");
    }
}
