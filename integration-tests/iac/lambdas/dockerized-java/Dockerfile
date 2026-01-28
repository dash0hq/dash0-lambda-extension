ARG EXTENSION_IMAGE=dash0/extension-java:latest

FROM ${EXTENSION_IMAGE} AS extension

FROM public.ecr.aws/lambda/java:21

# Copy extension from extension image
COPY --from=extension /opt /opt

# Copy handler JAR
COPY app.jar ${LAMBDA_TASK_ROOT}/app.jar

# Enable the extension wrapper
ENV AWS_LAMBDA_EXEC_WRAPPER=/opt/wrapper

CMD ["com.example.App::handleRequest"]
