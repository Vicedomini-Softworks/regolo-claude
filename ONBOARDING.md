# Regolo Claude — Onboarding

## What this is

CLI tool that lets you run Claude Code against Regolo.ai models instead of Anthropic's API. It bundles a proxy that translates between the Anthropic Messages API (what Claude Code expects) and Regolo's OpenAI-compatible Chat Completions API.

## Prerequisites

- Rust stable (`curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`)
- Claude Code (`npm install -g @anthropic-ai/claude-code`)
- A Regolo.ai API key (get one at regolo.ai)

## Setup

```bash
git clone git@github.com:Vicedomini-Softworks/regolo-claude.git
cd regolo-claude
make install          # builds release binary, installs to /usr/local/bin/regolo
regolo login          # paste your Regolo API key when prompted
```

## Run it

```bash
regolo claude                      # launches Claude Code with default model (brick-v1-beta)
regolo claude --model qwen3:32B    # specific model
regolo list                        # see all available models
```

That's it. The proxy starts automatically in the background — no separate terminal needed.

## Project structure

```
src/main.rs          CLI commands + proxy server (single binary)
Cargo.toml           Rust dependencies
Makefile             build / install / test targets
.github/workflows/
  ci.yml             build + test on every push to main
  release.yml        cross-compile + publish binaries on git tags
```

## Development workflow

```bash
make build    # debug build
make test     # run tests
make fmt      # rustfmt
make lint     # clippy
make release  # optimized build → target/release/regolo
```

## Releasing

Tag a commit and push — CI builds binaries for macOS (arm64 + x86_64) and Linux (x86_64 + arm64) and publishes them as a GitHub Release.

```bash
git tag v0.2.0
git push origin v0.2.0
```

## Troubleshooting

**`claude` not found** — install Claude Code: `npm install -g @anthropic-ai/claude-code`

**Auth errors** — re-run `regolo login` or set `REGOLO_API_KEY=<key>` in your environment

**Unexpected model responses** — run with `DEBUG=1 regolo claude` to see full request/response logs
