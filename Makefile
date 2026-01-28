ZIP_NAME_PYTHON = layer-lrap-python.zip
ZIP_NAME_NODE = layer-lrap-node.zip
ZIP_NAME_JAVA = layer-lrap-java.zip
ZIP_NAME_MANUAL = layer-lrap-manual.zip
LAYER_NAME_PYTHON = dash0-extension-python
LAYER_NAME_NODE = dash0-extension-node
LAYER_NAME_JAVA = dash0-extension-java
LAYER_NAME_MANUAL = dash0-extension-manual
LAMBDA_LAYER_MARKER_PYTHON := .lambda-layer-python
LAMBDA_LAYER_MARKER_NODE := .lambda-layer-node
LAMBDA_LAYER_MARKER_JAVA := .lambda-layer-java
LAMBDA_LAYER_MARKER_MANUAL := .lambda-layer-manual
CARGO_FEATURES :=
PYTHON_DEPS_IMAGE := lrap-python-deps

# Docker/ECR image settings
AWS_ACCOUNT_ID := $(shell aws sts get-caller-identity --query Account --output text)
AWS_REGION := $(shell aws configure get region)
ECR_REGISTRY := $(AWS_ACCOUNT_ID).dkr.ecr.$(AWS_REGION).amazonaws.com
ECR_REPO_PYTHON ?= dash0-extension-python
DOCKER_IMAGE_PYTHON := $(ECR_REGISTRY)/$(ECR_REPO_PYTHON)
VERSION ?= latest

#-- current-condition vars
# Check if Docker is available or running-- needed by `cargo cross`.
#    modify if not cross-compiling or if using different tooling
DOCKER_RUNNING := $(shell docker ps > /dev/null 2>&1 && echo -n yes)
RS_FILES := $(shell find src -name "*.rs")


.phony: build clean cargo zip clean-build clean-cargo deploy-layer doc python node java manual docker-python docker-push-python

# * Build both x86_64 and aarch64 binaries
# * create a Layer '.zip'
# * use AWS CLI to publish Lambda layer
#
default: python node java manual

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


build/$(ZIP_NAME_NODE): build/lrap_x86_64 build/lrap_aarch64 opt/entrypoint opt/node/package.json opt/node/wrapper opt/node/webpack.config.mjs opt/node/init.mjs
	@echo Building Node.js layer
	@rm -f build/$(ZIP_NAME_NODE)
	@rm -rf build/stage-node
	@mkdir -p build/stage-node/extensions
	@cp build/lrap_x86_64 build/stage-node/
	@cp build/lrap_aarch64 build/stage-node/
	@cp opt/entrypoint build/stage-node/extensions/lrap
	@cp opt/node/wrapper build/stage-node/wrapper
	@cd opt/node && npm install && npm run build
	@cp opt/node/dist/init.mjs build/stage-node/
	@# Copy only the external dependencies needed at runtime
	@mkdir -p build/stage-node/node_modules
	@cp -r opt/node/node_modules/require-in-the-middle build/stage-node/node_modules/
	@cp -r opt/node/node_modules/import-in-the-middle build/stage-node/node_modules/
	@cp -r opt/node/node_modules/module-details-from-path build/stage-node/node_modules/
	@cp -r opt/node/node_modules/debug build/stage-node/node_modules/
	@cp -r opt/node/node_modules/ms build/stage-node/node_modules/
	@cp -r opt/node/node_modules/acorn build/stage-node/node_modules/
	@cp -r opt/node/node_modules/acorn-import-attributes build/stage-node/node_modules/ 2>/dev/null || true
	@cp -r opt/node/node_modules/cjs-module-lexer build/stage-node/node_modules/
	@cd build/stage-node && zip -r ../$(ZIP_NAME_NODE) *


build/$(ZIP_NAME_JAVA): build/lrap_x86_64 build/lrap_aarch64 opt/entrypoint opt/java/wrapper
	@echo Building Java layer
	@rm -f build/$(ZIP_NAME_JAVA)
	@rm -rf build/stage-java
	@mkdir -p build/stage-java/extensions
	@mkdir -p build/stage-java/java/lib
	@cp build/lrap_x86_64 build/stage-java/
	@cp build/lrap_aarch64 build/stage-java/
	@cp opt/entrypoint build/stage-java/extensions/lrap
	@cp opt/java/wrapper build/stage-java/wrapper
	@curl -L -o build/stage-java/java/lib/lumigo-opentelemetry.jar https://github.com/lumigo-io/opentelemetry-java-distro/releases/download/v0.19.1/lumigo-opentelemetry-0.19.1.jar
	@cd build/stage-java && zip -r ../$(ZIP_NAME_JAVA) *


build/$(ZIP_NAME_MANUAL): build/lrap_x86_64 build/lrap_aarch64 opt/entrypoint opt/manual/wrapper
	@echo Building Manual layer
	@rm -f build/$(ZIP_NAME_MANUAL)
	@rm -rf build/stage-manual
	@mkdir -p build/stage-manual/extensions
	@cp build/lrap_x86_64 build/stage-manual/
	@cp build/lrap_aarch64 build/stage-manual/
	@cp opt/entrypoint build/stage-manual/extensions/lrap
	@cp opt/manual/wrapper build/stage-manual/wrapper
	@cd build/stage-manual && zip -r ../$(ZIP_NAME_MANUAL) *


python: $(LAMBDA_LAYER_MARKER_PYTHON)

node: $(LAMBDA_LAYER_MARKER_NODE)

java: $(LAMBDA_LAYER_MARKER_JAVA)

manual: $(LAMBDA_LAYER_MARKER_MANUAL)

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

$(LAMBDA_LAYER_MARKER_JAVA): build/$(ZIP_NAME_JAVA)
	@echo "Publishing Lambda Extension to layer \"$(LAYER_NAME_JAVA)\""
	@aws lambda publish-layer-version --layer-name $(LAYER_NAME_JAVA) --zip-file fileb://build/$(ZIP_NAME_JAVA) \
		--description "Layer to intercept and sanitize Lambda input and output data. Compatible with all runtimes" \
		--compatible-architectures x86_64 arm64 --no-cli-pager
	@touch $(LAMBDA_LAYER_MARKER_JAVA)

$(LAMBDA_LAYER_MARKER_MANUAL): build/$(ZIP_NAME_MANUAL)
	@echo "Publishing Lambda Extension to layer \"$(LAYER_NAME_MANUAL)\""
	@aws lambda publish-layer-version --layer-name $(LAYER_NAME_MANUAL) --zip-file fileb://build/$(ZIP_NAME_MANUAL) \
		--description "Layer to intercept and sanitize Lambda input and output data. Compatible with all runtimes" \
		--compatible-architectures x86_64 arm64 --no-cli-pager
	@touch $(LAMBDA_LAYER_MARKER_MANUAL)




doc:
	@cargo doc
	@echo
	@echo "Docs are located in target/doc/aws_lambda_runtime_api_proxy_rs/index.html"


# =============================================================================
# Docker targets for containerized Lambda deployments
# =============================================================================

# Build and push Docker image for Python extension to ECR
# Usage: make docker-python VERSION=1.0.0
docker-python: build/lrap_x86_64 build/lrap_aarch64 build/python
	@echo "Creating ECR repository $(ECR_REPO_PYTHON) if it doesn't exist..."
	@aws ecr describe-repositories --repository-names $(ECR_REPO_PYTHON) 2>/dev/null || \
		aws ecr create-repository --repository-name $(ECR_REPO_PYTHON) --no-cli-pager
	@echo "Logging in to ECR..."
	@aws ecr get-login-password --region $(AWS_REGION) | docker login --username AWS --password-stdin $(ECR_REGISTRY)
	@echo "Building Docker image $(DOCKER_IMAGE_PYTHON):$(VERSION)"
	@docker build -f docker/Dockerfile.python -t $(DOCKER_IMAGE_PYTHON):$(VERSION) .
	@echo "Pushing Docker image $(DOCKER_IMAGE_PYTHON):$(VERSION)"
	@docker push $(DOCKER_IMAGE_PYTHON):$(VERSION)
	@echo ""
	@echo "Usage in your Dockerfile:"
	@echo "  COPY --from=$(DOCKER_IMAGE_PYTHON):$(VERSION) /opt /opt"
	@echo "  ENV AWS_LAMBDA_EXEC_WRAPPER=/opt/wrapper"
