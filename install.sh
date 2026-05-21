#!/bin/bash
# Installation script for Regolo CLI wrapper

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REGOLO_SCRIPT="$SCRIPT_DIR/regolo"

# Check if running on macOS/Linux
if [[ "$OSTYPE" == "msys" || "$OSTYPE" == "win32" ]]; then
    echo "Error: This script is designed for macOS/Linux"
    exit 1
fi

# Find a suitable installation path
if [[ -d "$HOME/.local/bin" ]]; then
    INSTALL_DIR="$HOME/.local/bin"
elif [[ -d "$HOME/bin" ]]; then
    INSTALL_DIR="$HOME/bin"
else
    INSTALL_DIR="$HOME/.local/bin"
    mkdir -p "$INSTALL_DIR"
fi

# Copy the script
cp "$REGOLO_SCRIPT" "$INSTALL_DIR/regolo"
chmod +x "$INSTALL_DIR/regolo"

# Install Python dependencies if pip is available
if command -v pip3 &> /dev/null; then
    echo "Installing Python dependencies..."
    pip3 install -q keyring aiohttp 2>/dev/null || pip install -q keyring aiohttp 2>/dev/null || true
elif command -v python3 &> /dev/null; then
    echo "Installing Python dependencies..."
    python3 -m pip install -q keyring aiohttp 2>/dev/null || python3 -m pip install --user -q keyring aiohttp 2>/dev/null || true
fi

echo "✓ Regolo CLI installed to $INSTALL_DIR/regolo"

# Check if it's in PATH
if ! command -v regolo &> /dev/null; then
    echo ""
    echo "⚠ Warning: $INSTALL_DIR is not in your PATH"
    echo ""
    echo "To fix this, add one of the following to your shell config file:"
    echo ""
    
    if [[ -f "$HOME/.zshrc" ]]; then
        echo "  echo 'export PATH=\"$INSTALL_DIR:\$PATH\"' >> ~/.zshrc"
    elif [[ -f "$HOME/.bashrc" ]]; then
        echo "  echo 'export PATH=\"$INSTALL_DIR:\$PATH\"' >> ~/.bashrc"
    fi
    
    echo ""
    echo "Then run: source ~/.zshrc (or ~/.bashrc)"
else
    echo "✓ regolo is now available in your PATH"
fi

echo ""
echo "Next steps:"
echo "  1. Set your API key: export REGOLO_API_KEY=your_api_key"
echo "  2. Run: regolo list"
echo "  3. Launch Claude: regolo claude --model=qwen3.5:122B"
echo ""
echo "The proxy server will start automatically when you run 'regolo claude'."
