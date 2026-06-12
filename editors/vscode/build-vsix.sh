#!/usr/bin/env bash
# Build the mastdiff-search VS Code extension as an installable .vsix.
#
# Usage:
#   ./build-vsix.sh            # build
#   ./build-vsix.sh --install  # build, then install into local VS Code
#
# Requires Node.js + npm. vsce is fetched on demand via npx, no global
# install needed.
set -euo pipefail
cd "$(dirname "$0")"

INSTALL=false
for arg in "$@"; do
    case "$arg" in
        --install) INSTALL=true ;;
        *) echo "unknown option: $arg" >&2; exit 2 ;;
    esac
done

if [ ! -d node_modules ]; then
    echo "── installing devDependencies ──"
    npm install
fi

echo "── compiling TypeScript ──"
npm run compile

echo "── packaging .vsix ──"
# "echo y" auto-confirms the "no LICENSE found" prompt.
echo y | npx --yes @vscode/vsce package

VSIX=$(ls -t mastdiff-search-*.vsix | head -1)
echo
echo "Built: $(pwd)/$VSIX"

if $INSTALL; then
    echo "── installing into VS Code ──"
    code --install-extension "$VSIX"
    echo "Installed. Reload VS Code windows to pick up the new version."
else
    echo "Install with: code --install-extension $VSIX"
fi
