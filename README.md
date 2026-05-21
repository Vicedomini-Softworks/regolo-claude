# Regolo Claude CLI

Single Rust binary that launches Claude Code with Regolo.ai models. Includes a built-in proxy that translates between Anthropic's Messages API (what Claude Code speaks) and Regolo's OpenAI-compatible Chat Completions API.

## Installation

### From release (recommended)

Download the binary for your platform from [Releases](https://github.com/Vicedomini-Softworks/regolo-claude/releases), then:

```bash
tar xzf regolo-<target>.tar.gz
install -m 755 regolo-<target> /usr/local/bin/regolo
```

### Build from source

```bash
cargo build --release
# or
make install
```

Requires Rust stable. Claude Code must be installed separately:

```bash
npm install -g @anthropic-ai/claude-code
```

## Authentication

```bash
regolo login    # prompts for API key, stores in ~/.regolo/auth.json
regolo logout   # removes stored key
```

Or set `REGOLO_API_KEY` in your environment to skip the file entirely.

## Usage

```bash
regolo claude                        # launch Claude Code (default model: brick-v1-beta)
regolo claude --model qwen3:32B      # specify model
regolo list                          # list available Regolo models
regolo proxy                         # start proxy in foreground (port 0 = random)
regolo proxy --port 8789             # start proxy on fixed port
```

`regolo claude` automatically starts the proxy in the background, wires up the env vars, and launches `claude`. No manual proxy management needed.

## How it works

```
Claude Code  →  POST /v1/messages (Anthropic format)
                ↓
            regolo proxy
                ↓
            POST /v1/chat/completions (OpenAI format)
                ↓
            api.regolo.ai
```

The proxy handles:
- Anthropic `tools` / `tool_use` / `tool_result` ↔ OpenAI `tools` / `tool_calls` / `tool` role
- `system` top-level field → system message
- Response translated back to Anthropic Messages API format (`type: message`, `stop_reason`, etc.)

## Environment Variables

| Variable | Description |
|---|---|
| `REGOLO_API_KEY` | API key (overrides `~/.regolo/auth.json`) |
| `DEBUG` | Set to `1` to log full request/response to stdout |

## Models

Default: `brick-v1-beta`

Run `regolo list` to see all available models.

## Development

```bash
make build    # debug build
make release  # release build
make test     # run tests
make fmt      # format
make lint     # clippy
make install  # install to /usr/local/bin
```

CI runs on every push to `main`. Tagged releases (`v*`) build binaries for:
- `aarch64-apple-darwin`
- `x86_64-apple-darwin`
- `x86_64-unknown-linux-gnu`
- `aarch64-unknown-linux-gnu`
