FROM public.ecr.aws/lambda/python:3.12 AS build

WORKDIR /var/task

# Install git (required for git+ dependencies in requirements.txt)
RUN dnf install -y git && dnf clean all

# Install sub-dependencies
COPY opt/python/distro/requirements.txt .
RUN python3.12 -m pip install --upgrade --no-cache-dir pip \
    && python3.12 -m pip install --no-cache-dir -r requirements.txt -t /asset-output/python

# Copy the distro package source directly
COPY opt/python/distro/src/lumigo_opentelemetry /asset-output/python/lumigo_opentelemetry
