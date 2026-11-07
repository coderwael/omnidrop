#!/usr/bin/env bash
# One-command helper to initialize Git, push the OmniDrop workspace to GitHub,
# and trigger the automated cloud binary build & release workflow.
set -euo pipefail

if [ $# -lt 1 ]; then
  echo "Usage: ./push_to_github_cloud_build.sh <https://github.com/YOUR_USERNAME/omnidrop.git> [v1.0.0]"
  exit 1
fi

REMOTE_URL="$1"
RELEASE_TAG="${2:-v1.0.0}"
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

cd "$ROOT_DIR"

if [ ! -d ".git" ]; then
  git init
  git branch -M main
fi

git add -A
git commit -m "OmniDrop Full-Stack Workspace + Cloud Release Workflow" || true

git remote remove origin 2>/dev/null || true
git remote add origin "$REMOTE_URL"

git tag -f "$RELEASE_TAG"
git push -u origin main --force
git push origin "$RELEASE_TAG" --force

echo "========================================================================"
echo "Pushed to $REMOTE_URL with tag $RELEASE_TAG!"
echo "Open your repository's 'Actions' tab or 'Releases' page on GitHub"
echo "to download OmniDrop-Android-arm64.apk and OmniDrop-Windows-x64.zip."
echo "========================================================================"
