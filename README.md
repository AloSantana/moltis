<div align="center">

<a href="https://moltis.org"><img src="https://raw.githubusercontent.com/moltis-org/moltis/main/website/favicon.svg" alt="Moltis" width="64"></a>

# Moltis — A Rust-native claw you can trust

One binary — sandboxed, secure, yours.

[![CI](https://github.com/moltis-org/moltis/actions/workflows/ci.yml/badge.svg)](https://github.com/moltis-org/moltis/actions/workflows/ci.yml)
[![codecov](https://codecov.io/gh/moltis-org/moltis/graph/badge.svg)](https://codecov.io/gh/moltis-org/moltis)
[![CodSpeed](https://img.shields.io/endpoint?url=https://codspeed.io/badge.json&style=flat&label=CodSpeed)](https://codspeed.io/moltis-org/moltis)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/Rust-1.91%2B-orange.svg)](https://www.rust-lang.org)
[![Discord](https://img.shields.io/discord/1469505370169933837?color=5865F2&label=Discord&logo=discord&logoColor=white)](https://discord.gg/XnmrepsXp5)

[Installation](#installation) • [Quick Start](#quick-start) • [Web UI](#web-ui--remote-access) • [Multi-Agent Orchestration](#multi-agent-orchestration) • [Comparison](#comparison) • [Architecture](#architecture--crate-map) • [Security](#security) • [Features](#features) • [Contributing](CONTRIBUTING.md)

</div>

---

Moltis recently hit [the front page of Hacker News](https://news.ycombinator.com/item?id=46993587). Please [open an issue](https://github.com/moltis-org/moltis/issues) for any friction at all. I'm focused on making Moltis excellent.

**Secure by design** — Your keys never leave your machine. Every command runs in a sandboxed container, never on your host.

**Your hardware** — Runs on a Mac Mini, a Raspberry Pi, or any server you own. One Rust binary, no Node.js, no npm, no runtime.

**Full-featured** — Voice, memory, scheduling, Telegram, Discord, browser automation, MCP servers — all built-in. No plugin marketplace to get supply-chain attacked through.

**Multi-agent orchestration** — 12 specialised agent roles (Architect, Debug Detective, DevOps, and more) automatically route, execute in parallel, and hand off context — fully built into the binary.

**Auditable** — The agent loop + provider model fits in ~5K lines. The core (excluding the optional web UI) is ~196K lines across 46 modular crates you can audit independently, with 3,100+ tests and zero `unsafe` code\*.

## Installation

### One-liner (macOS / Linux)

```bash
curl -fsSL https://www.moltis.org/install.sh | sh
```

### Homebrew

```bash
brew install moltis-org/tap/moltis
```

### Docker (recommended for servers)

```bash
docker pull ghcr.io/moltis-org/moltis:latest
```

### Build from source

Requires Rust 1.91+ and [just](https://github.com/casey/just).

```bash
git clone https://github.com/AloSantana/moltis.git
cd moltis
just build-css        # Build Tailwind CSS for the web UI
just build-release    # Optimised release binary
./target/release/moltis
```

For a full release with WASM sandbox tools:

```bash
just build-release-with-wasm
./target/release/moltis
```

Or install directly from crates.io:

```bash
cargo install moltis --git https://github.com/moltis-org/moltis
```

---

## Quick Start

### Local (binary)

```bash
moltis                                   # Start on https://localhost:13131
moltis --port 8080                       # Custom port
moltis --config-dir /path/to/config \   # Custom config + data dirs
       --data-dir /path/to/data
```

On first run a **setup code** is printed to the terminal. Open the URL shown and
enter it to set your password or register a passkey. After that the setup code is
cleared and normal auth applies.

### Local (Docker)

```bash
docker run -d \
  --name moltis \
  -p 13131:13131 \
  -v moltis-config:/home/moltis/.config/moltis \
  -v moltis-data:/home/moltis/.moltis \
  -v /var/run/docker.sock:/var/run/docker.sock \
  ghcr.io/moltis-org/moltis:latest
```

Open `https://localhost:13131` and complete the setup. See [Docker docs](https://docs.moltis.org/docker.html) for Podman, OrbStack, TLS trust, and volume persistence.

### Cloud Deployment

| Provider | One-click deploy |
|----------|--------|
| DigitalOcean | [![Deploy to DO](https://www.deploytodo.com/do-btn-blue.svg)](https://cloud.digitalocean.com/apps/new?repo=https://github.com/moltis-org/moltis/tree/main) |

**Fly.io** (CLI):

```bash
fly launch --image ghcr.io/moltis-org/moltis:latest
fly secrets set MOLTIS_PASSWORD="your-password"
fly deploy
```

**Railway / Render** — `railway.json` and `render.yaml` are included in the repo.  
All cloud configs use `--no-tls` because the platform handles TLS termination.  
See [Cloud Deploy docs](https://docs.moltis.org/cloud-deploy.html) for full details.

### Configure your first LLM provider

Add to `~/.moltis/moltis.toml` (or edit via the web UI → Settings):

```toml
[providers.anthropic]
api_key = "sk-ant-..."

[providers.openai]
api_key = "sk-..."

# Or a local model via Ollama
[providers.ollama]
base_url = "http://localhost:11434"
```

---

## Web UI & Remote Access

Moltis ships a built-in web UI available immediately after startup — no separate
install, no npm, no build step required.

### Accessing the Web UI

| Scenario | URL |
|----------|-----|
| Local binary | `https://moltis.localhost:13131` or `https://localhost:13131` |
| Docker | `https://localhost:13131` |
| Remote server | `https://<your-server-ip>:13131` |
| Custom port | `https://localhost:<port>` |
| Tailscale | `https://<machine-name>.ts.net:13131` |
| Fly.io / DigitalOcean | `https://<app-name>.fly.dev` |

> **TLS note** — Moltis auto-generates a self-signed certificate on first run.
> Your browser will warn you the first time; add a permanent exception, or point
> `cert_path` / `key_path` at your own certificate in `moltis.toml`.

### Remote access via Tailscale (recommended)

Tailscale provides zero-config, authenticated remote access without opening
firewall ports:

```toml
# moltis.toml
[tailscale]
enabled = true
mode    = "serve"   # serve (HTTPS on port 443) or funnel (public internet)
```

```bash
moltis                        # Moltis registers with your tailnet automatically
# Access from any device on your tailnet:
# https://<hostname>.ts.net
```

### Remote access via reverse proxy (nginx / Caddy)

```nginx
# nginx — place behind Cloudflare or your own TLS termination
server {
    listen 443 ssl;
    server_name moltis.example.com;

    location / {
        proxy_pass         https://localhost:13131;
        proxy_http_version 1.1;
        proxy_set_header   Upgrade $http_upgrade;
        proxy_set_header   Connection "upgrade";
        proxy_set_header   Host $host;
    }
}
```

```caddyfile
# Caddyfile — automatic HTTPS
moltis.example.com {
    reverse_proxy localhost:13131 {
        transport http { tls_insecure_skip_verify }
    }
}
```

Set `MOLTIS_BEHIND_PROXY=true` when running behind a proxy so Moltis trusts
forwarded headers correctly.

### Authentication modes

| Mode | How to enable |
|------|---------------|
| Password + passkey (default) | Set on first run via the setup code |
| API key (for programmatic access) | Generate in web UI → Settings → API Keys |
| Auth disabled (local dev only) | `moltis.toml`: `[auth] disabled = true` |
| Reset password | `moltis auth reset-password` |
| Reset all credentials | `moltis auth reset-identity` |

---

## Multi-Agent Orchestration

Moltis includes a built-in multi-agent orchestration system that **automatically
routes tasks to the most appropriate specialised agent**, runs agents in parallel,
and transfers context between them via typed handoffs.

> Enabled by default — no configuration required to start using it.

### Agent Roles

12 specialised roles are built in:

| Role | Best for |
|------|----------|
| 🚀 **Rapid Implementer** | Fast autonomous code implementation |
| 🏛 **Architect** | System architecture and design |
| 🔍 **Debug Detective** | Debugging and root-cause analysis |
| 🔬 **Deep Researcher** | Comprehensive research and analysis |
| 🌐 **Full-Stack Developer** | Complete web-application development |
| 🐳 **DevOps & Infra** | Docker, Kubernetes, CI/CD pipelines |
| 🧪 **Testing Expert** | Testing and validation |
| ⚡ **Performance Optimizer** | Performance profiling and optimisation |
| 🛡 **Code Reviewer** | Security and quality code reviews |
| 📚 **Docs Master** | Documentation creation |
| 🔧 **Repo Optimizer** | Repository setup and tooling |
| 🔌 **API Developer** | API design and implementation |

### Smart Routing

Tasks are routed automatically by keyword scoring — no manual role selection needed:

```bash
# REST API
curl -X POST https://localhost:13131/api/agents/route \
  -H "Authorization: Bearer <api-key>" \
  -H "Content-Type: application/json" \
  -d '{ "task": "debug the crash in the login flow" }'
# → routes to Debug Detective

curl -X POST https://localhost:13131/api/agents/route \
  -d '{ "task": "write integration tests for the auth module" }'
# → routes to Testing Expert

curl -X POST https://localhost:13131/api/agents/route \
  -d '{ "task": "create a docker-compose CI pipeline" }'
# → routes to DevOps & Infra
```

### Execute a Task

```bash
# Let the orchestrator pick the best agent automatically
curl -X POST https://localhost:13131/api/agents/execute \
  -H "Authorization: Bearer <api-key>" \
  -H "Content-Type: application/json" \
  -d '{ "task": "optimise the database query bottleneck" }'

# Force a specific agent role
curl -X POST https://localhost:13131/api/agents/execute \
  -d '{ "task": "review this PR for security issues", "role": "code_reviewer" }'
```

### Parallel Orchestration Plans

Run multiple agents concurrently, then synthesise results:

```bash
# Comprehensive Analysis: deep-researcher + architect + code-reviewer → docs-master synthesises
curl -X POST https://localhost:13131/api/agents/execute \
  -d '{ "task": "analyse the authentication module", "plan": "comprehensive_analysis" }'

# Feature Review: rapid-implementer + code-reviewer → testing-expert synthesises
curl -X POST https://localhost:13131/api/agents/execute \
  -d '{ "task": "add OAuth2 login", "plan": "feature_review" }'

# Security Audit: code-reviewer + debug-detective → architect synthesises
curl -X POST https://localhost:13131/api/agents/execute \
  -d '{ "task": "audit the payment processing code", "plan": "security_audit" }'
```

### Agent Status & History

```bash
# List all agents with live statistics (task count, success rate, mean latency)
curl https://localhost:13131/api/agents/status \
  -H "Authorization: Bearer <api-key>"

# View handoff history for the current session
curl https://localhost:13131/api/agents/history \
  -H "Authorization: Bearer <api-key>"
```

### WebSocket RPC

The same functionality is available over the WebSocket connection used by the web UI:

```jsonc
// agents.roles.list — read scope
{ "method": "agents.roles.list" }

// agents.roles.route — read scope
{ "method": "agents.roles.route", "params": { "task": "write tests" } }

// agents.roles.execute — write scope
{ "method": "agents.roles.execute", "params": { "task": "build a REST API", "plan": "feature_review" } }
```

### Configuration

```toml
# moltis.toml
[agents.orchestration]
enabled              = true
routing_strategy     = "hybrid"   # "keyword" | "intent" | "hybrid"
max_concurrent_agents = 4
agent_timeout_secs   = 120
disabled_roles       = []         # e.g. ["docs_master", "repo_optimizer"]
```

Disable the feature entirely (opt-out):

```toml
# moltis.toml
[agents.orchestration]
enabled = false
```

Or at build time (removes the code entirely):

```bash
cargo build --no-default-features --features "agent,caldav,tls,web-ui"
```

---

## Usage Examples

### Chat via web UI

Open `https://localhost:13131` and start chatting. The agent picks tools,
executes them in a sandbox, and streams results back.

### Chat via Telegram / Discord / MS Teams

Configure a channel in `moltis.toml` and the agent responds to messages in your
channel with full tool access:

```toml
[channels.telegram]
token = "your-bot-token"

[channels.discord]
token = "your-bot-token"
```

### Scheduled tasks (cron)

```toml
[[cron]]
name     = "daily-report"
schedule = "0 9 * * *"          # Every day at 09:00
task     = "Summarise open PRs and post to #standup on Slack"
```

### Memory & long-term context

Moltis automatically reads and writes per-agent `MEMORY.md` files and supports
vector-search over past conversations:

```toml
[agents.presets.coder]
system_prompt_suffix = "You are an expert Rust engineer."

[agents.presets.coder.memory]
scope     = "project"   # project-local MEMORY.md
max_lines = 400
```

### Agent presets / sub-agents

```toml
[agents.presets.reviewer]
model                = "claude-opus-4-5"
system_prompt_suffix = "Focus on security and correctness."
delegate_only        = false

[agents.presets.reviewer.tools]
allow = ["read_file", "grep", "list_files"]
deny  = ["exec", "write_file"]
```

### Voice I/O

```toml
[voice.tts]
provider = "openai"

[voice.stt]
provider = "whisper"
```

```bash
moltis                  # The web UI's mic button activates voice mode
```

### MCP servers

```toml
[mcp.servers.github]
command = ["npx", "-y", "@modelcontextprotocol/server-github"]
env     = { GITHUB_PERSONAL_ACCESS_TOKEN = "ghp_..." }
```

---

## Comparison

| | OpenClaw | PicoClaw | NanoClaw | ZeroClaw | **Moltis** |
|---|---|---|---|---|---|
| Language | TypeScript | Go | TypeScript | Rust | **Rust** |
| Agent loop | ~430K LoC | Small | ~500 LoC | ~3.4K LoC | **~5K LoC** (`runner.rs` + `model.rs`) |
| Full codebase | — | — | — | 1,000+ tests | **~124K LoC** (2,300+ tests) |
| Runtime | Node.js + npm | Single binary | Node.js | Single binary (3.4 MB) | **Single binary (44 MB)** |
| Sandbox | App-level | — | Docker | Docker | **Docker + Apple Container** |
| Memory safety | GC | GC | GC | Ownership | **Ownership, zero `unsafe`\*** |
| Auth | Basic | API keys | None | Token + OAuth | **Password + Passkey + API keys + Vault** |
| Voice I/O | Plugin | — | — | — | **Built-in (15+ providers)** |
| MCP | Yes | — | — | — | **Yes (stdio + HTTP/SSE)** |
| Hooks | Yes (limited) | — | — | — | **15 event types** |
| Skills | Yes (store) | Yes | Yes | Yes | **Yes (+ OpenClaw Store)** |
| Memory/RAG | Plugin | — | Per-group | SQLite + FTS | **SQLite + FTS + vector** |
| Multi-agent orchestration | — | — | — | — | **12 roles, parallel plans, typed handoffs** |

\* `unsafe` is denied workspace-wide. The only exceptions are opt-in FFI wrappers behind the `local-embeddings` feature flag, not part of the core.

> [Full comparison with benchmarks →](https://docs.moltis.org/comparison.html)

## Architecture — Crate Map

**Core** (always compiled):

| Crate | LoC | Role |
|-------|-----|------|
| `moltis` (cli) | 4.0K | Entry point, CLI commands |
| `moltis-agents` | 9.6K | Agent loop, streaming, prompt assembly, orchestration |
| `moltis-providers` | 17.6K | LLM provider implementations |
| `moltis-gateway` | 36.1K | HTTP/WS server, RPC, auth |
| `moltis-chat` | 11.5K | Chat engine, agent orchestration |
| `moltis-tools` | 21.9K | Tool execution, sandbox |
| `moltis-config` | 7.0K | Configuration, validation |
| `moltis-sessions` | 3.8K | Session persistence |
| `moltis-plugins` | 1.9K | Hook dispatch, plugin formats |
| `moltis-service-traits` | 1.3K | Shared service interfaces |
| `moltis-common` | 1.1K | Shared utilities |
| `moltis-protocol` | 0.8K | Wire protocol types |

**Optional** (feature-gated or additive):

| Category | Crates | Combined LoC |
|----------|--------|-------------|
| Web UI | `moltis-web` | 4.5K |
| GraphQL | `moltis-graphql` | 4.8K |
| Voice | `moltis-voice` | 6.0K |
| Memory | `moltis-memory`, `moltis-qmd` | 5.9K |
| Channels | `moltis-telegram`, `moltis-whatsapp`, `moltis-discord`, `moltis-msteams`, `moltis-channels` | 14.9K |
| Browser | `moltis-browser` | 5.1K |
| Scheduling | `moltis-cron`, `moltis-caldav` | 5.2K |
| Extensibility | `moltis-mcp`, `moltis-skills`, `moltis-wasm-tools` | 9.1K |
| Auth & Security | `moltis-auth`, `moltis-oauth`, `moltis-onboarding`, `moltis-vault` | 6.6K |
| Networking | `moltis-network-filter`, `moltis-tls`, `moltis-tailscale` | 3.5K |
| Provider setup | `moltis-provider-setup` | 4.3K |
| Import | `moltis-openclaw-import` | 7.6K |
| Apple native | `moltis-swift-bridge` | 2.1K |
| Metrics | `moltis-metrics` | 1.7K |
| Other | `moltis-projects`, `moltis-media`, `moltis-routing`, `moltis-canvas`, `moltis-auto-reply`, `moltis-schema-export`, `moltis-benchmarks` | 2.5K |

Use `--no-default-features --features lightweight` for constrained devices (Raspberry Pi, etc.).

## Security

- **Zero `unsafe` code\*** — denied workspace-wide; only opt-in FFI behind `local-embeddings` flag
- **Sandboxed execution** — Docker + Apple Container, per-session isolation
- **Secret handling** — `secrecy::Secret`, zeroed on drop, redacted from tool output
- **Authentication** — password + passkey (WebAuthn), rate-limited, per-IP throttle
- **SSRF protection** — DNS-resolved, blocks loopback/private/link-local
- **Origin validation** — rejects cross-origin WebSocket upgrades
- **Hook gating** — `BeforeToolCall` hooks can inspect/block any tool invocation

See [Security Architecture](https://docs.moltis.org/security.html) for details.

## Features

- **Multi-Agent Orchestration** — 12 specialised agent roles, smart keyword routing, parallel execution plans (Comprehensive Analysis, Feature Review, Security Audit), typed context handoffs
- **AI Gateway** — Multi-provider LLM support (OpenAI Codex, GitHub Copilot, Local), streaming responses, agent loop with sub-agent delegation, parallel tool execution
- **Communication** — Web UI, Telegram, Microsoft Teams, Discord, API access, voice I/O (8 TTS + 7 STT providers), mobile PWA with push notifications
- **Memory & Context** — Per-agent memory workspaces, embeddings-powered long-term memory, hybrid vector + full-text search, session persistence with auto-compaction, project context
- **Extensibility** — MCP servers (stdio + HTTP/SSE), skill system, 15 lifecycle hook events with circuit breaker, destructive command guard
- **Security** — Encryption-at-rest vault (XChaCha20-Poly1305 + Argon2id), password + passkey + API key auth, sandbox isolation, SSRF/CSWSH protection
- **Operations** — Cron scheduling, OpenTelemetry tracing, Prometheus metrics, cloud deploy (Fly.io, DigitalOcean), Tailscale integration

## How It Works

Moltis is a **local-first AI gateway** — a single Rust binary that sits
between you and multiple LLM providers. Everything runs on your machine; no
cloud relay required.

```
┌─────────────┐  ┌─────────────┐  ┌─────────────┐
│   Web UI    │  │  Telegram   │  │  Discord    │
└──────┬──────┘  └──────┬──────┘  └──────┬──────┘
       │                │                │
       └────────┬───────┴────────┬───────┘
                │   WebSocket    │
                ▼                ▼
        ┌─────────────────────────────────┐
        │          Gateway Server         │
        │   (Axum · HTTP · WS · Auth)     │
        ├─────────────────────────────────┤
        │     Multi-Agent Orchestrator    │
        │  ┌──────────┐  ┌─────────────┐  │
        │  │  Router  │→ │ Agent Roles │  │
        │  │(keywords)│  │ (12 roles)  │  │
        │  └──────────┘  └──────┬──────┘  │
        │                       │         │
        │        Chat Service             │
        │  ┌───────────┐ ┌─────────────┐  │
        │  │   Agent   │ │    Tool     │  │
        │  │   Runner  │◄┤   Registry  │  │
        │  └─────┬─────┘ └─────────────┘  │
        │        │                        │
        │  ┌─────▼─────────────────────┐  │
        │  │    Provider Registry      │  │
        │  │  Multiple providers       │  │
        │  │  (Codex · Copilot · Local)│  │
        │  └───────────────────────────┘  │
        ├─────────────────────────────────┤
        │  Sessions  │ Memory  │  Hooks   │
        │  (JSONL)   │ (SQLite)│ (events) │
        └─────────────────────────────────┘
                       │
               ┌───────▼───────┐
               │    Sandbox    │
               │ Docker/Apple  │
               │  Container    │
               └───────────────┘
```

See [Quickstart](https://docs.moltis.org/quickstart.html) for gateway startup, message flow, sessions, and memory details.

## Star History

[![Star History Chart](https://api.star-history.com/svg?repos=moltis-org/moltis&type=date&legend=top-left)](https://www.star-history.com/#moltis-org/moltis&type=date&legend=top-left)

## License

MIT
