#!/bin/bash
# Sets up ECR repository policy to allow CDK image publishing role to pull images.
# This must run BEFORE `cdk deploy` because Docker builds happen during asset publishing.

set -e

ECR_REPOS=("dash0-extension-python" "dash0-extension-node")
CDK_QUALIFIER="hnb659fds"

ACCOUNT_ID=$(aws sts get-caller-identity --query Account --output text)
REGION="${AWS_REGION:-${AWS_DEFAULT_REGION:-$(aws configure get region)}}"

echo "Setting up ECR policy for account $ACCOUNT_ID in region $REGION"

# Set repository policy to allow CDK image publishing role
CDK_ROLE="arn:aws:iam::${ACCOUNT_ID}:role/cdk-${CDK_QUALIFIER}-image-publishing-role-${ACCOUNT_ID}-${REGION}"

for ECR_REPO_NAME in "${ECR_REPOS[@]}"; do
  echo "Setting policy for $ECR_REPO_NAME..."
  aws ecr set-repository-policy \
    --repository-name "$ECR_REPO_NAME" \
    --policy-text "{\"Version\":\"2012-10-17\",\"Statement\":[{\"Sid\":\"AllowCDKPull\",\"Effect\":\"Allow\",\"Principal\":{\"AWS\":\"$CDK_ROLE\"},\"Action\":[\"ecr:BatchGetImage\",\"ecr:GetDownloadUrlForLayer\"]}]}"
done

echo "ECR policy set for Allowed principal: $CDK_ROLE"
