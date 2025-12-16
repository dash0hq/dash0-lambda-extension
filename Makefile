# 
# Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
# SPDX-License-Identifier: MIT-0
#
#
#-- config vars
ZIP_NAME_PYTHON = layer-lrap-python.zip
ZIP_NAME_NODE = layer-lrap-node.zip
LAYER_NAME_PYTHON = lrap-python
LAYER_NAME_NODE = lrap-node
LAMBDA_LAYER_MARKER_PYTHON := .lambda-layer-python
LAMBDA_LAYER_MARKER_NODE := .lambda-layer-node
CARGO_FEATURES := 
PYTHON_DEPS_IMAGE := lrap-python-deps

#-- current-condition vars
# Check if Docker is available or running-- needed by `cargo cross`.
#    modify if not cross-compiling or if using different tooling
DOCKER_RUNNING := $(shell docker ps > /dev/null 2>&1 && echo -n yes)
RS_FILES := $(shell find src -name "*.rs")


.phony: build clean cargo zip clean-build clean-cargo deploy-layer doc python node

# * Build both x86_64 and aarch64 binaries
# * create a Layer '.zip'
# * use AWS CLI to publish Lambda layer
#
default: python node

clean: clean-build clean-cargo

clean-build: 
	@rm -rf build
	@rm -f .lambda-layer*

clean-cargo:
	@cargo clean

build/lrap_x86_64: $(RS_FILES) Cargo.toml
	@echo Building Rust application for x86_64
	@mkdir -p build
	@cross build --release --target x86_64-unknown-linux-musl ${CARGO_FEATURES}
	@cp target/x86_64-unknown-linux-musl/release/aws-lambda-runtime-api-proxy-rs build/lrap_x86_64

build/lrap_aarch64: $(RS_FILES) Cargo.toml
	@echo Building Rust application for aarch64
	@mkdir -p build
	@cross build --release --target aarch64-unknown-linux-musl ${CARGO_FEATURES}
	@cp target/aarch64-unknown-linux-musl/release/aws-lambda-runtime-api-proxy-rs build/lrap_aarch64

build/python: opt/python/requirements.txt opt/python/Dockerfile
	@mkdir -p build
	@rm -rf build/python
	@docker build -t $(PYTHON_DEPS_IMAGE) -f opt/python/Dockerfile .
	@cid=$$(docker create $(PYTHON_DEPS_IMAGE)); \
		docker cp $$cid:/asset-output/python build/python; \
		docker rm $$cid >/dev/null



build/$(ZIP_NAME_PYTHON): build/lrap_x86_64 build/lrap_aarch64 opt/entrypoint opt/python/wrapper opt/python/otel_wrapper.py build/python
	@rm -f build/$(ZIP_NAME_PYTHON)
	@rm -rf build/stage-python
	@mkdir -p build/stage-python/extensions
	@cp build/lrap_x86_64 build/stage-python/
	@cp build/lrap_aarch64 build/stage-python/
	@cp opt/entrypoint build/stage-python/extensions/lrap
	@cp opt/python/wrapper build/stage-python/wrapper
	@cp -r build/python build/stage-python/python
	@cp opt/python/otel_wrapper.py build/stage-python/python/otel_wrapper.py
	@cd build/stage-python && zip -r ../$(ZIP_NAME_PYTHON) *


build/$(ZIP_NAME_NODE): build/lrap_x86_64 build/lrap_aarch64 opt/entrypoint opt/node/package.json opt/node/wrapper opt/node/init.mjs
	@echo Building Node.js layer
	@rm -f build/$(ZIP_NAME_NODE)
	@rm -rf build/stage-node
	@mkdir -p build/stage-node/extensions
	@cp build/lrap_x86_64 build/stage-node/
	@cp build/lrap_aarch64 build/stage-node/
	@cp opt/entrypoint build/stage-node/extensions/lrap
	@cp opt/node/wrapper build/stage-node/wrapper
	@cd opt/node && npm install
	@cp -r opt/node/node_modules build/stage-node/
	@cp opt/node/init.mjs build/stage-node/
	@cd build/stage-node && zip -r ../$(ZIP_NAME_NODE) *


python: $(LAMBDA_LAYER_MARKER_PYTHON)

node: $(LAMBDA_LAYER_MARKER_NODE)

$(LAMBDA_LAYER_MARKER_PYTHON): build/$(ZIP_NAME_PYTHON)
	@echo "Publishing Lambda Extension to layer \"$(LAYER_NAME_PYTHON)\""
	@aws lambda publish-layer-version --layer-name $(LAYER_NAME_PYTHON) --zip-file fileb://build/$(ZIP_NAME_PYTHON) \
		--description "Layer to intercept and sanitize Lambda input and output data. Compatible with all runtimes" \
		--compatible-architectures x86_64 arm64 --no-cli-pager
	@touch $(LAMBDA_LAYER_MARKER_PYTHON)

$(LAMBDA_LAYER_MARKER_NODE): build/$(ZIP_NAME_NODE)
	@echo "Publishing Lambda Extension to layer \"$(LAYER_NAME_NODE)\""
	@aws lambda publish-layer-version --layer-name $(LAYER_NAME_NODE) --zip-file fileb://build/$(ZIP_NAME_NODE) \
		--description "Layer to intercept and sanitize Lambda input and output data. Compatible with all runtimes" \
		--compatible-architectures x86_64 arm64 --no-cli-pager
	@touch $(LAMBDA_LAYER_MARKER_NODE)




doc: 
	@cargo doc
	@echo
	@echo "Docs are located in target/doc/aws_lambda_runtime_api_proxy_rs/index.html"
