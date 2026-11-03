package org.example;

import com.amazonaws.services.lambda.runtime.Context;
import com.amazonaws.services.lambda.runtime.RequestHandler;

import java.net.URI;
import java.net.http.HttpClient;
import java.net.http.HttpRequest;
import java.net.http.HttpResponse;

/**
 * Runs long enough for the extension to flush while the invocation is still
 * running (the stack sets DASH0_FLUSH_INTERVAL=1000), and logs before and
 * after a child span, so logs are sent before the handler span exists.
 */
public class EarlyFlushHandler implements RequestHandler<Object, String> {

    @Override
    public String handleRequest(Object input, Context context) {
        System.out.println("early-flush: before child span");
        try {
            HttpResponse<Void> response = HttpClient.newHttpClient().send(
                    HttpRequest.newBuilder(URI.create("https://jsonplaceholder.typicode.com/posts/1")).build(),
                    HttpResponse.BodyHandlers.discarding());
            Thread.sleep(2500);
            System.out.println("early-flush: after child span " + response.statusCode());
            Thread.sleep(1500);
        } catch (Exception e) {
            throw new RuntimeException(e);
        }
        System.out.println("early-flush: done");
        return "done";
    }
}
