package org.example;

import com.amazonaws.services.lambda.runtime.Context;
import com.amazonaws.services.lambda.runtime.RequestHandler;
import java.util.ArrayList;
import java.util.List;
import java.util.Map;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.Future;
import java.util.concurrent.atomic.AtomicLong;
import software.amazon.awssdk.core.sync.RequestBody;
import software.amazon.awssdk.http.urlconnection.UrlConnectionHttpClient;
import software.amazon.awssdk.services.s3.S3Client;
import software.amazon.awssdk.services.sqs.SqsClient;

/**
 * Imitates a long "full resync": for every change it writes the change to S3,
 * reads it back and publishes it to SQS. With the Java agent each AWS SDK call
 * is a span, so `items` changes produce about 3 x `items` spans in one
 * invocation.
 *
 * <p>Input: {"items": 100000, "threads": 32}
 */
public class HighLoadHandler implements RequestHandler<Map<String, Object>, String> {

  private static final String BUCKET = System.getenv("BUCKET");
  private static final String QUEUE_URL = System.getenv("QUEUE_URL");
  private static final S3Client S3 =
      S3Client.builder().httpClient(UrlConnectionHttpClient.create()).build();
  private static final SqsClient SQS =
      SqsClient.builder().httpClient(UrlConnectionHttpClient.create()).build();

  @Override
  public String handleRequest(Map<String, Object> input, Context context) {
    int items = ((Number) input.getOrDefault("items", 1000)).intValue();
    int threads = ((Number) input.getOrDefault("threads", 32)).intValue();
    // A changed record of the size a typical resync writes.
    String payload = "{\"change\":\"" + "x".repeat(1500) + "\"}";
    AtomicLong done = new AtomicLong();
    long start = System.currentTimeMillis();

    ExecutorService pool = Executors.newFixedThreadPool(threads);
    List<Future<?>> futures = new ArrayList<>();
    for (int t = 0; t < threads; t++) {
      int worker = t;
      futures.add(
          pool.submit(
              () -> {
                for (int i = worker; i < items; i += threads) {
                  // Overwrite a small key set so the bucket does not grow.
                  String key = "changes/" + (i % 256) + ".json";
                  S3.putObject(b -> b.bucket(BUCKET).key(key), RequestBody.fromString(payload));
                  S3.getObjectAsBytes(b -> b.bucket(BUCKET).key(key));
                  int id = i;
                  SQS.sendMessage(b -> b.queueUrl(QUEUE_URL).messageBody("change-" + id));
                  long n = done.incrementAndGet();
                  if (n % 10000 == 0) {
                    System.out.println("processed " + n + " changes; rss " + processRss());
                  }
                }
              }));
    }
    for (Future<?> f : futures) {
      try {
        f.get();
      } catch (Exception e) {
        throw new RuntimeException(e);
      }
    }
    pool.shutdown();
    String result =
        "processed " + done.get() + " changes in " + (System.currentTimeMillis() - start) + " ms";
    System.out.println(result);
    return result;
  }

  /** Resident memory of every process in the sandbox, e.g. "java=812MB dash0=21MB". */
  private static String processRss() {
    StringBuilder sb = new StringBuilder();
    java.io.File[] procs = new java.io.File("/proc").listFiles((d, n) -> n.matches("\\d+"));
    if (procs == null) {
      return "n/a";
    }
    for (java.io.File p : procs) {
      try {
        String name = "";
        long rssKb = 0;
        for (String line : java.nio.file.Files.readAllLines(p.toPath().resolve("status"))) {
          if (line.startsWith("Name:")) name = line.substring(5).trim();
          if (line.startsWith("VmRSS:")) rssKb = Long.parseLong(line.replaceAll("\\D", ""));
        }
        if (rssKb > 1024) sb.append(name).append('=').append(rssKb / 1024).append("MB ");
      } catch (Exception ignored) {
        // process exited or not readable
      }
    }
    return sb.toString().trim();
  }
}
