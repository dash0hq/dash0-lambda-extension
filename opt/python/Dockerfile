FROM public.ecr.aws/lambda/python:3.12 AS build

WORKDIR /var/task

COPY opt/python/opentelemetry_exporter_otlp_proto_http-1.39.1-py3-none-any.whl /Users/mosheshaham/workspace/opentelemetry-python/exporter/opentelemetry-exporter-otlp-proto-http/dist/opentelemetry_exporter_otlp_proto_http-1.39.1-py3-none-any.whl
COPY opt/python/lumigo_opentelemetry-1.0.199.tar.gz .
COPY opt/python/requirements.txt .

# Install dependencies into a directory Lambda will recognize when added to a layer.
RUN python3.12 -m pip install --upgrade --no-cache-dir pip \
    && python3.12 -m pip install --no-cache-dir -r requirements.txt -t /asset-output/python
