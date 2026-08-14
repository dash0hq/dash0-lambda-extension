ARG EXTENSION_IMAGE=public.ecr.aws/dash0/extension-node:latest

FROM ${EXTENSION_IMAGE} AS extension

FROM public.ecr.aws/lambda/nodejs:22

# Copy extension from extension image
COPY --from=extension /opt /opt

# Copy handler
COPY handler.js ${LAMBDA_TASK_ROOT}

# Enable the extension wrapper
ENV AWS_LAMBDA_EXEC_WRAPPER=/opt/wrapper

CMD ["handler.handler"]
