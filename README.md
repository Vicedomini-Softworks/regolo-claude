# Regolo Claude CLI

CLI wrapper that launches Claude Code with Regolo.ai models via OpenAI-compatible API.

## Installation

```bash
./install.sh
pip install -r requirements.txt
```

## Authentication

```bash
regolo login      # Enter API key (stored in ~/.regolo/auth.json)
regolo logout     # Remove stored API key
```

## Usage

### List Available Models

```bash
regolo list
```

### Launch Claude Directly (v1/completion API)

```bash
regolo claude --model=brick-v1-beta
```

### Launch Claude with Messages API Proxy (Recommended)

Since Regolo only supports `/v1/completion` but Claude expects `/v1/messages`, use the proxy:

```bash
# Terminal 1: Start the proxy server
regolo proxy

# Terminal 2: Launch Claude
regolo claude --model=brick-v1-beta
```

Or manually:

```bash
# Terminal 1
python router_server.py

# Terminal 2
ANTHROPIC_BASE_URL=http://localhost:8789 ANTHROPIC_API_KEY=your_key claude
```

## Proxy Server

The `router_server.py` translates between:
- **Claude's format**: `/v1/messages` (chat-based)
- **Regolo's format**: `/v1/completion` (prompt-based)

### Proxy Endpoints

- `POST /v1/messages` - Translated to `/v1/completion`
- `GET /v1/models` - Forwarded to Regolo
- `GET /health` - Health check

## Environment Variables

- `REGOLO_API_KEY` - Your Regolo API key (optional, can use `regolo login`)
- `PORT` - Proxy server port (default: 8789)

## Models

Default: `brick-v1-beta`

Other options: `qwen3.5:122B`, `qwen3.5:72B`, `qwen3:32B`, `qwen3:14B`, `qwen3:8B`

## Prerequisites

1. Python 3.9+
2. `keyring` package (for secure credential storage)
3. `aiohttp` package (for proxy server)
4. Claude Code installed globally: `npm install -g @anthropic-ai/claude-code`

## Troubleshooting

### 403 Errors
Usually caused by wrong BASE_URL. Should be `https://api.regolo.ai`, not `https://api.regolo.ai/v1`

### Authentication Errors
Check API key is valid via `regolo login` or `export REGOLO_API_KEY=...`

### Proxy Connection Errors
Ensure the proxy server is running before launching Claude:
```bash
regolo proxy
```
