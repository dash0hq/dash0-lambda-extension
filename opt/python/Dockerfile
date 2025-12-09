FROM public.ecr.aws/lambda/python:3.12 AS build

WORKDIR /var/task

COPY opt/python/requirements.txt .

# Install dependencies into a directory Lambda will recognize when added to a layer.
RUN python3.12 -m pip install --upgrade --no-cache-dir pip \
    && python3.12 -m pip install --no-cache-dir -r requirements.txt -t /asset-output/python
