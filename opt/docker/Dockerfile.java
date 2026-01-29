# Dockerfile for building the Java Lambda extension image
#
# This image contains the extension files at /opt and can be used in
# multi-stage builds for containerized Lambda functions:
#
#   FROM public.ecr.aws/lambda/java:21
#   COPY --from=dash0/extension-java:latest /opt /opt
#   ENV AWS_LAMBDA_EXEC_WRAPPER=/opt/wrapper
#   ...
#
# Build with:
#   make docker-java
#
# Or manually:
#   # First build the binaries
#   make build/lrap_x86_64 build/lrap_aarch64
#   # Then build the Docker image
#   docker build -f opt/docker/Dockerfile.java -t dash0/extension-java:latest .

# Stage 1: Download the Java agent JAR
FROM public.ecr.aws/lambda/java:21 AS downloader

RUN curl -L -o /tmp/lumigo-opentelemetry.jar \
    https://github.com/lumigo-io/opentelemetry-java-distro/releases/download/v0.19.1/lumigo-opentelemetry-0.19.1.jar

# Stage 2: Final image with extension
FROM scratch

# Copy extension binaries (both architectures - entrypoint selects at runtime)
COPY build/lrap_x86_64 /opt/lrap_x86_64
COPY build/lrap_aarch64 /opt/lrap_aarch64

# Copy entrypoint script (Lambda extension entry point)
COPY opt/entrypoint /opt/extensions/lrap

# Copy wrapper script
COPY opt/java/wrapper /opt/wrapper

# Copy Java agent JAR
COPY --from=downloader /tmp/lumigo-opentelemetry.jar /opt/java/lib/lumigo-opentelemetry.jar
