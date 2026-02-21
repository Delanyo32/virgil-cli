#!/usr/bin/env bash
set -euo pipefail

# Load env vars
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/.env"

# Source folder to upload
SOURCE_DIR="../vigil-frontend"

# Install/check aws cli availability
if ! command -v aws &> /dev/null; then
  echo "Error: aws CLI is not installed. Install it with: brew install awscli"
  exit 1
fi

# Export credentials for aws CLI
export AWS_ACCESS_KEY_ID="$S3_ACCESS_KEY_ID"
export AWS_SECRET_ACCESS_KEY="$S3_SECRET_ACCESS_KEY"

# Upload to S3 (Cloudflare R2)
aws s3 sync "$SOURCE_DIR" "s3://$S3_BUCKET_NAME/vigil-frontend/" \
  --endpoint-url "$S3_ENDPOINT" \
  --exclude ".git/*" \
  --exclude "node_modules/*" \
  --exclude ".output/*" \
  --exclude ".nitro/*" \
  --exclude ".DS_Store"

echo "Upload complete."
