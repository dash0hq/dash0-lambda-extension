# High-load reproducer

A Java 25 Lambda that imitates a long, high-volume invocation: for each of
`items` changes it calls S3 PutObject, S3 GetObject and SQS SendMessage from
`threads` worker threads. With the Java agent each call is a span, so 100k
changes produce about 300k spans in one invocation. Every 10k changes it logs
the resident memory of each process in the sandbox (JVM and extension).

`run.sh` deploys two copies of the function, each with its own Java layer zip
(typically one built from `master` and one from a branch), with 2048 MB and
`OTEL_METRIC_EXPORT_INTERVAL=100`.

```bash
cd benchmarks/high-load
./gradlew buildZip
export AWS_PROFILE=<profile> DASH0_TOKEN=<token> DASH0_DATASET=<dataset>
./run.sh deploy <master-layer.zip> <branch-layer.zip>
./run.sh invoke master 100000 64          # items threads [memoryMB]
./run.sh invoke branch 100000 64
./run.sh destroy
```

`AWS_REGION` defaults to `eu-central-1`, `RESOURCE_PREFIX` to `rm-hl-`, and
`DASH0_ENDPOINT` to the dev ingress. Build the layer zips with
`make build/layer-dash0-java.zip` on each revision.

Result for the fix in #127 (100k changes, 2048 MB): `master` ran out of memory
after ~40k changes with the extension at 363 MB; the branch finished with the
extension at ~11 MB and delivered ~300k spans.
