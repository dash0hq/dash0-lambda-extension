#!/bin/bash
# Reproduces a long, high-volume Java invocation (customer report: ~5 min,
# several hundred thousand AWS SDK spans) against two Java layers, typically one
# built from master and one from a branch.
#
# Usage:
#   run.sh deploy <master-layer.zip> <branch-layer.zip>
#   run.sh invoke <master|branch> [items] [threads] [memoryMB]
#   run.sh destroy
#
# Needs: AWS credentials (AWS_PROFILE), DASH0_TOKEN (exported by Lambda
# functions to DASH0_ENDPOINT), and `./gradlew buildZip` run first.
set -euo pipefail
cd "$(dirname "$0")"

export AWS_REGION=${AWS_REGION:-eu-central-1}
P=${RESOURCE_PREFIX:-rm-hl-}
ACCOUNT=$(aws sts get-caller-identity --query Account --output text)
BUCKET=${P}${ACCOUNT}-${AWS_REGION}
ROLE=${P}role
DASH0_ENDPOINT=${DASH0_ENDPOINT:-https://ingress.eu-west-1.aws.dash0-dev.com:4318}
DASH0_DATASET=${DASH0_DATASET:-default}

publish_layer() { # name zip
  aws lambda publish-layer-version --layer-name "$1" --zip-file "fileb://$2" \
    --compatible-architectures x86_64 arm64 --query LayerVersionArn --output text
}

deploy() {
  aws s3api create-bucket --bucket "$BUCKET" \
    --create-bucket-configuration LocationConstraint="$AWS_REGION" >/dev/null
  QUEUE_URL=$(aws sqs create-queue --queue-name ${P}changes \
    --attributes MessageRetentionPeriod=60 --query QueueUrl --output text)
  QUEUE_ARN=$(aws sqs get-queue-attributes --queue-url "$QUEUE_URL" \
    --attribute-names QueueArn --query Attributes.QueueArn --output text)

  ROLE_ARN=$(aws iam create-role --role-name "$ROLE" --query Role.Arn --output text \
    --assume-role-policy-document '{"Version":"2012-10-17","Statement":[{"Effect":"Allow","Principal":{"Service":"lambda.amazonaws.com"},"Action":"sts:AssumeRole"}]}')
  aws iam attach-role-policy --role-name "$ROLE" \
    --policy-arn arn:aws:iam::aws:policy/service-role/AWSLambdaBasicExecutionRole
  aws iam put-role-policy --role-name "$ROLE" --policy-name data --policy-document "{
    \"Version\":\"2012-10-17\",\"Statement\":[
      {\"Effect\":\"Allow\",\"Action\":[\"s3:PutObject\",\"s3:GetObject\"],\"Resource\":\"arn:aws:s3:::$BUCKET/*\"},
      {\"Effect\":\"Allow\",\"Action\":\"sqs:SendMessage\",\"Resource\":\"$QUEUE_ARN\"}]}"
  sleep 10 # IAM propagation

  for variant in master branch; do
    zip=$1; [ $variant = branch ] && zip=$2
    layer=$(publish_layer ${P}dash0-extension-java-$variant "$zip")
    env=$(python3 -c 'import json,os,sys; print(json.dumps({"Variables": {
      "AWS_LAMBDA_EXEC_WRAPPER": "/opt/wrapper",
      "OTEL_METRIC_EXPORT_INTERVAL": "100",
      "DASH0_ENDPOINT": os.environ["DASH0_ENDPOINT"],
      "DASH0_DATASET": os.environ["DASH0_DATASET"],
      "DASH0_TOKEN": os.environ["DASH0_TOKEN"],
      "BUCKET": sys.argv[1], "QUEUE_URL": sys.argv[2]}}))' "$BUCKET" "$QUEUE_URL")
    aws lambda create-function --function-name ${P}$variant --runtime java25 \
      --architectures x86_64 --memory-size 2048 --timeout 900 \
      --handler org.example.HighLoadHandler::handleRequest --role "$ROLE_ARN" \
      --zip-file fileb://build/distributions/high-load.zip --layers "$layer" \
      --environment "$env" --query FunctionArn --output text
  done
  aws lambda wait function-active-v2 --function-name ${P}master
  aws lambda wait function-active-v2 --function-name ${P}branch
}

invoke() { # variant items threads memory
  local fn=${P}$1
  if [ -n "${4:-}" ]; then
    aws lambda update-function-configuration --function-name "$fn" --memory-size "$4" >/dev/null
    aws lambda wait function-updated-v2 --function-name "$fn"
  fi
  local out; out=$(mktemp)
  aws lambda invoke --function-name "$fn" --cli-read-timeout 960 --log-type Tail \
    --payload "{\"items\":${2:-100000},\"threads\":${3:-32}}" --cli-binary-format raw-in-base64-out \
    --query '[FunctionError,LogResult]' --output text "$out" |
    awk '{print "FunctionError: " $1; print $2}' | { read -r err; echo "$err"; base64 -d 2>/dev/null; }
  echo "Response: $(cat "$out")"
  rm -f "$out"
}

destroy() {
  for variant in master branch; do
    aws lambda delete-function --function-name ${P}$variant || true
    for v in $(aws lambda list-layer-versions --layer-name ${P}dash0-extension-java-$variant \
        --query 'LayerVersions[].Version' --output text); do
      aws lambda delete-layer-version --layer-name ${P}dash0-extension-java-$variant --version-number "$v"
    done
  done
  aws logs delete-log-group --log-group-name /aws/lambda/${P}master 2>/dev/null || true
  aws logs delete-log-group --log-group-name /aws/lambda/${P}branch 2>/dev/null || true
  QUEUE_URL=$(aws sqs get-queue-url --queue-name ${P}changes --query QueueUrl --output text 2>/dev/null) &&
    aws sqs delete-queue --queue-url "$QUEUE_URL" || true
  aws s3 rb "s3://$BUCKET" --force || true
  aws iam delete-role-policy --role-name "$ROLE" --policy-name data || true
  aws iam detach-role-policy --role-name "$ROLE" \
    --policy-arn arn:aws:iam::aws:policy/service-role/AWSLambdaBasicExecutionRole || true
  aws iam delete-role --role-name "$ROLE" || true
}

cmd=$1; shift
export DASH0_ENDPOINT DASH0_DATASET
$cmd "$@"
