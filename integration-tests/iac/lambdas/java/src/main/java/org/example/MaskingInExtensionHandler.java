package org.example;

import com.amazonaws.services.lambda.runtime.Context;
import com.amazonaws.services.lambda.runtime.RequestHandler;

import java.net.URI;
import java.net.http.HttpClient;
import java.net.http.HttpRequest;
import java.net.http.HttpResponse;

/**
 * Sends a URL query the Java distro does not mask (the upstream instrumentation redacts only a
 * fixed list of signature parameters). With DASH0_MASK_IN_EXTENSION=true the extension masks
 * it with the default patterns.
 */
public class MaskingInExtensionHandler implements RequestHandler<Object, String> {

    private static final String SECRET = "XQZ-IT-SECRET";

    @Override
    public String handleRequest(Object input, Context context) {
        try {
            HttpResponse<Void> response = HttpClient.newHttpClient().send(
                    HttpRequest.newBuilder(URI.create(
                            "https://jsonplaceholder.typicode.com/posts/1?api_token=" + SECRET + "&page=1"))
                            .GET()
                            .build(),
                    HttpResponse.BodyHandlers.discarding());
            return "status " + response.statusCode();
        } catch (Exception e) {
            throw new RuntimeException(e);
        }
    }
}
