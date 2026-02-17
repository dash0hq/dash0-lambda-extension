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
#   # First build the binaries and Java distro
#   make build/lrap_x86_64 build/lrap_aarch64
#   cd opt/java/opentelemetry-java-distro && ./gradlew -Pversion=1.0.0-SNAPSHOT assemble -x javadoc
#   # Then build the Docker image
#   docker build -f opt/docker/Dockerfile.java -t dash0/extension-java:latest .

FROM scratch

# Copy extension binaries (both architectures - entrypoint selects at runtime)
COPY build/lrap_x86_64 /opt/lrap_x86_64
COPY build/lrap_aarch64 /opt/lrap_aarch64

# Copy entrypoint script (Lambda extension entry point)
COPY opt/entrypoint /opt/extensions/lrap

# Copy wrapper script
COPY opt/java/wrapper /opt/wrapper

# Copy Java agent JAR (built locally from opt/java/opentelemetry-java-distro)
COPY opt/java/opentelemetry-java-distro/agent/build/libs/agent-1.0.0-SNAPSHOT-all.jar /opt/java/lib/lumigo-opentelemetry.jar
