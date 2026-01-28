#!/bin/bash
# Sets up ECR repository policy to allow CDK image publishing role to pull images.
# This must run BEFORE `cdk deploy` because Docker builds happen during asset publishing.

set -e

ECR_REPO_NAME="dash0-extension-python"
CDK_QUALIFIER="hnb659fds"

ACCOUNT_ID=$(aws sts get-caller-identity --query Account --output text)
REGION="${AWS_REGION:-${AWS_DEFAULT_REGION:-$(aws configure get region)}}"

echo "Setting up ECR policy for account $ACCOUNT_ID in region $REGION"

# Check if repository exists
if ! aws ecr describe-repositories --repository-names "$ECR_REPO_NAME" 2>/dev/null; then
  echo "Repository $ECR_REPO_NAME does not exist yet. It will be created by 'make docker-python'."
  exit 0
fi

# Set repository policy to allow CDK image publishing role
CDK_ROLE="arn:aws:iam::${ACCOUNT_ID}:role/cdk-${CDK_QUALIFIER}-image-publishing-role-${ACCOUNT_ID}-${REGION}"

aws ecr set-repository-policy \
  --repository-name "$ECR_REPO_NAME" \
  --policy-text "{\"Version\":\"2012-10-17\",\"Statement\":[{\"Sid\":\"AllowCDKPull\",\"Effect\":\"Allow\",\"Principal\":{\"AWS\":\"$CDK_ROLE\"},\"Action\":[\"ecr:BatchGetImage\",\"ecr:GetDownloadUrlForLayer\"]}]}"

echo "ECR policy set for repository $ECR_REPO_NAME"
echo "Allowed principal: $CDK_ROLE"
