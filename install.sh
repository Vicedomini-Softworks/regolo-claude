#!/bin/bash
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
INSTALL_DIR="/usr/local/bin"

if [[ "$OSTYPE" == "msys" || "$OSTYPE" == "win32" ]]; then
    echo "Error: This script is for macOS/Linux only." >&2
    exit 1
fi

if ! command -v cargo &> /dev/null; then
    echo "Error: Rust is not installed." >&2
    echo "Install it: curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh" >&2
    exit 1
fi

echo "Building regolo..."
cd "$SCRIPT_DIR"
make install INSTALL="$INSTALL_DIR"

echo ""
echo "✓ regolo installed to $INSTALL_DIR/regolo"
echo ""
echo "Next steps:"
echo "  regolo login                     # set your Regolo API key"
echo "  regolo list                      # list available models"
echo "  regolo claude --model qwen3:32B  # launch Claude Code"
