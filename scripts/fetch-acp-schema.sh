#!/usr/bin/env sh
# Re-fetch the vendored ACP feed schema, unmodified, and print its SHA-256.
# Usage: scripts/fetch-acp-schema.sh [version-folder]   (default 2026-04-17)
set -eu
VERSION="${1:-2026-04-17}"
BASE="https://raw.githubusercontent.com/agentic-commerce-protocol/agentic-commerce-protocol/main"
DEST="rules/acp-feed/vendor"
mkdir -p "$DEST"
curl -fsSL "$BASE/spec/$VERSION/json-schema/schema.feed.json" -o "$DEST/schema.feed.json"
curl -fsSL "$BASE/LICENSE" -o "$DEST/LICENSE"
if command -v sha256sum >/dev/null 2>&1; then
  sha256sum "$DEST/schema.feed.json"
else
  shasum -a 256 "$DEST/schema.feed.json"
fi
echo "Now update $DEST/NOTICE.md and the checked date in rules/acp-feed/."
