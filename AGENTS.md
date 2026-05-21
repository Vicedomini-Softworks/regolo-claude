# Regolo CLI Wrapper - AGENTS.md

## Project Overview
CLI wrapper that launches Claude Code with Regolo.ai models via OpenAI-compatible API.

## Key Commands
```bash
# Installation
./install.sh

# Authentication
regolo login      # Enter API key (stored in ~/.regolo/auth.json)
regolo logout     # Remove stored API key

# Operations
regolo list       # List available models from API
regolo claude --model=<model>   # Launch Claude Code with specified model
```

## Critical API Configuration
- **Base URL**: `https://api.regolo.ai` (NOT `/v1` - that's part of endpoint paths)
- **Auth header**: `Authorization: Bearer <api_key>`
- **API key sources** (in order):
  1. `REGOLO_API_KEY` environment variable
  2. `~/.regolo/auth.json` file

## Model Names
Default: `brick-v1-beta`
Other options: `qwen3.5:122B`, `qwen3.5:72B`, `qwen3:32B`, `qwen3:14B`, `qwen3:8B`

## Environment Variables Set When Launching Claude
```bash
ANTHROPIC_BASE_URL=https://api.regolo.ai
ANTHROPIC_API_KEY=<api_key>
ANTHROPIC_MODEL=<model_name>
```

## Common Issues
- **403 errors**: Usually caused by wrong BASE_URL (should be `https://api.regolo.ai`, not `https://api.regolo.ai/v1`)
- **Auth failures**: Check API key is valid via `regolo login` or `export REGOLO_API_KEY=...`

## File Structure
```
regolo-claude/
├── regolo          # Main Python CLI script
├── install.sh      # Installation script (copies to ~/.local/bin)
├── requirements.txt # Python deps: keyring>=24.0.0
└── README.md
```

## Prerequisites
1. Python 3.x
2. `keyring` package (for secure credential storage)
3. Claude Code installed globally: `npm install -g @anthropic-ai/claude-code`
