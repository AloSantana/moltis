# Local Linux Installation Guide

This guide covers installing Moltis on a local Linux machine so it acts as your primary hub — the central node that runs the AI agent loop, serves the Web UI, and optionally connects remote machines (including DigitalOcean Droplets) as worker nodes.

## Table of Contents

- [Prerequisites](#prerequisites)
- [Installation Methods](#installation-methods)
  - [Quick Install (Recommended)](#1-quick-install-recommended)
  - [Debian / Ubuntu (.deb)](#2-debian--ubuntu-deb)
  - [Fedora / RHEL (.rpm)](#3-fedora--rhel-rpm)
  - [Arch Linux](#4-arch-linux)
  - [Snap](#5-snap)
  - [AppImage](#6-appimage)
  - [Build from Source](#7-build-from-source)
- [First Run](#first-run)
- [Configure a Provider](#configure-a-provider)
- [Run as a System Service](#run-as-a-system-service)
- [Connecting a DigitalOcean WebUI Hub](#connecting-a-digitalocean-webui-hub)
- [Adding Remote Nodes](#adding-remote-nodes)
- [Verifying Everything Works](#verifying-everything-works)
- [Updating](#updating)
- [Troubleshooting](#troubleshooting)

---

## Prerequisites

| Requirement | Minimum | Notes |
|-------------|---------|-------|
| Linux kernel | 4.x or later | Ubuntu 20.04+, Debian 11+, Fedora 37+, Arch (rolling) |
| Architecture | `x86_64` or `aarch64` | ARM64 supported |
| Docker (optional) | 20.10+ | Required for sandboxed tool execution |
| Disk space | 200 MB | For the binary and initial data |
| Open port | 13131 (default) | Configurable; used by the web UI and WebSocket |

Docker is **optional** but strongly recommended — without it, the agent can still chat and call MCP tools, but sandboxed shell execution (running code in isolated containers) is disabled.

---

## Installation Methods

### 1. Quick Install (Recommended)

The fastest path: downloads the latest release binary for your platform.

```bash
curl -fsSL https://www.moltis.org/install.sh | sh
```

The script installs the binary to `~/.local/bin/moltis`. Make sure `~/.local/bin` is on your `PATH`:

```bash
# Add to ~/.bashrc or ~/.zshrc if not already present
export PATH="$HOME/.local/bin:$PATH"
source ~/.bashrc
```

Verify the installation:

```bash
moltis --version
```

### 2. Debian / Ubuntu (.deb)

```bash
# Download the latest .deb package
curl -LO https://github.com/moltis-org/moltis/releases/latest/download/moltis_amd64.deb

# Install
sudo dpkg -i moltis_amd64.deb
```

For ARM64 (Raspberry Pi, AWS Graviton, etc.):

```bash
curl -LO https://github.com/moltis-org/moltis/releases/latest/download/moltis_arm64.deb
sudo dpkg -i moltis_arm64.deb
```

### 3. Fedora / RHEL (.rpm)

```bash
curl -LO https://github.com/moltis-org/moltis/releases/latest/download/moltis.x86_64.rpm
sudo rpm -i moltis.x86_64.rpm
```

### 4. Arch Linux

```bash
curl -LO https://github.com/moltis-org/moltis/releases/latest/download/moltis.pkg.tar.zst
sudo pacman -U moltis.pkg.tar.zst
```

### 5. Snap

```bash
sudo snap install moltis
```

### 6. AppImage

```bash
curl -LO https://github.com/moltis-org/moltis/releases/latest/download/moltis.AppImage
chmod +x moltis.AppImage

# Optional: move to a location on your PATH
mv moltis.AppImage ~/.local/bin/moltis
```

### 7. Build from Source

Use this when you want the latest unreleased code or need to customise the build.

**Prerequisites:**

```bash
# Install Rust (if not already installed)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"

# Install build dependencies (Ubuntu/Debian)
sudo apt-get install -y build-essential pkg-config libssl-dev

# Install just (task runner)
cargo install just

# Install Node.js for building the Tailwind CSS bundle (Ubuntu/Debian)
sudo apt-get install -y nodejs npm
```

**Clone and build:**

```bash
git clone https://github.com/moltis-org/moltis.git
cd moltis
just build-css           # Build Tailwind CSS for the web UI
just build-release       # Release build (~5–10 min on first run)
```

**Install the built binary:**

```bash
cp target/release/moltis ~/.local/bin/moltis
```

---

## First Run

Start the gateway:

```bash
moltis
```

On first launch you will see something like:

```
🚀 Moltis gateway starting...
🔑 First-run setup code: XXXX-XXXX
🌐 Open http://localhost:13131 in your browser
```

1. Open `http://localhost:13131` in your browser.
2. Use the **setup code** printed to the terminal to complete authentication.
3. Set a password (used for future logins from non-localhost addresses).

> **Tip:** When accessing Moltis from the same machine, no password is needed — authentication is only enforced for remote access.

---

## Configure a Provider

You need at least one LLM provider before you can chat.

### Option A: Environment variable (fastest)

```bash
export ANTHROPIC_API_KEY="sk-ant-..."   # Anthropic Claude
export OPENAI_API_KEY="sk-..."          # OpenAI / ChatGPT
export GEMINI_API_KEY="..."             # Google Gemini
```

Restart Moltis — provider models appear automatically in the model picker.

### Option B: Web UI

1. Go to **Settings → Providers**
2. Enter your API key and click **Save**

### Option C: Local LLM (offline, no API key needed)

1. Install [Ollama](https://ollama.com/):
   ```bash
   curl -fsSL https://ollama.com/install.sh | sh
   ollama pull llama3
   ```
2. In Moltis: **Settings → Providers → Local LLM → Connect**

---

## Run as a System Service

To have Moltis start automatically on boot and restart on failure, install it as a
`systemd` user service.

### Create the service file

```bash
mkdir -p ~/.config/systemd/user

cat > ~/.config/systemd/user/moltis.service << 'EOF'
[Unit]
Description=Moltis AI Gateway
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
ExecStart=%h/.local/bin/moltis
Restart=on-failure
RestartSec=5s
# Forward stdout/stderr to the journal
StandardOutput=journal
StandardError=journal
# Environment (uncomment and set your provider key)
#Environment=ANTHROPIC_API_KEY=sk-ant-...

[Install]
WantedBy=default.target
EOF
```

### Enable and start

```bash
systemctl --user daemon-reload
systemctl --user enable --now moltis
```

### Check status and logs

```bash
systemctl --user status moltis
journalctl --user -u moltis -f
```

### Persist after logout (linger)

By default, user services stop when you log out. Enable lingering so Moltis keeps
running even when your session ends:

```bash
loginctl enable-linger "$USER"
```

---

## Connecting a DigitalOcean WebUI Hub

You can deploy a Moltis instance to a DigitalOcean Droplet (with persistent storage)
and connect your local Linux machine to it as a node, or use the Droplet as the
primary hub and your local machine as a worker node.

### Option A: Local machine is the hub; DigitalOcean is a worker node

Your local Moltis gateway is the hub. The DigitalOcean Droplet connects back as a node.

1. **On your local machine**, make Moltis reachable from the internet. The easiest
   option is [Tailscale](https://tailscale.com/):
   ```bash
   # Install Tailscale
   curl -fsSL https://tailscale.com/install.sh | sh
   sudo tailscale up

   # Note your Tailscale IP
   tailscale ip -4
   ```

2. **Generate a device token** in the Moltis Web UI:
   **Settings → Nodes → Generate Token**

3. **On the DigitalOcean Droplet**, install Moltis and register as a node:
   ```bash
   curl -fsSL https://www.moltis.org/install.sh | sh
   moltis node add \
     --host ws://<your-tailscale-ip>:13131/ws \
     --token <device-token> \
     --name "DigitalOcean Worker"
   ```

### Option B: DigitalOcean Droplet is the hub

Deploy a persistent Moltis instance on a Droplet using Docker Compose:

```bash
# On your DigitalOcean Droplet
# Install Docker first: https://docs.docker.com/engine/install/ubuntu/

# Download the DigitalOcean Compose file and start Moltis
curl -LO https://raw.githubusercontent.com/moltis-org/moltis/main/examples/docker-compose.digitalocean.yml
MOLTIS_PASSWORD=your-password docker compose -f docker-compose.digitalocean.yml up -d
```

4. **On your local Linux machine**, join as a node:
   ```bash
   moltis node add \
     --host wss://<droplet-public-ip>:13131/ws \
     --token <device-token> \
     --name "Local Linux Hub"
   ```

> **Note:** DigitalOcean App Platform does **not** support persistent disks for
> image-based services. Use a **Droplet** (VM) with Docker for persistent storage.
> See [Cloud Deploy docs](docs/src/cloud-deploy.md) for full deployment instructions.

---

## Adding Remote Nodes

Once your local Moltis is running, add any remote machine as a node:

```bash
# 1. Generate a token in the web UI: Settings → Nodes → Generate Token
#    Copy the full `moltis node add ...` command shown.

# 2. On the remote machine, run that command, e.g.:
moltis node add \
  --host ws://192.168.1.10:13131/ws \
  --token ot_abc123... \
  --name "Build Server"
```

The node installs a `systemd` user service and reconnects automatically on reboot.

### Useful node commands

```bash
moltis node status        # Show connection info
moltis node logs          # Print the log file path
moltis node remove        # Disconnect and remove the service
```

---

## Verifying Everything Works

### 1. Confirm the gateway is running

```bash
curl -s http://localhost:13131/health
# Expected: HTTP 200 with {"status":"ok"}
```

### 2. Check the web UI loads

Open `http://localhost:13131` in a browser. You should see the Moltis chat interface.

### 3. Send a test message

In the Chat tab:

```
You: What is 2 + 2?
```

The agent should respond with `4`.

### 4. Test tool execution (requires Docker)

```bash
# Ensure Docker is running
docker ps

# In the chat:
You: Create a file called test.txt containing "hello world" and show me its contents.
```

The agent should create the file inside a sandboxed container and display the output.

### 5. Verify nodes (if applicable)

```bash
moltis node list          # Lists all connected nodes
```

Or open **Settings → Nodes** in the web UI.

---

## Updating

### Binary (curl install)

```bash
curl -fsSL https://www.moltis.org/install.sh | sh
```

Running the install script again downloads the latest release and overwrites the binary.

### Package managers

```bash
# .deb
sudo dpkg -i moltis_amd64.deb    # re-download and reinstall

# Snap
sudo snap refresh moltis

# Homebrew (if installed via brew)
brew upgrade moltis
```

### From source

```bash
cd moltis
git pull
just build-css
just build-release
cp target/release/moltis ~/.local/bin/moltis
systemctl --user restart moltis
```

---

## Troubleshooting

### Port already in use

Moltis picks a random available port on first run and saves it. If port 13131 is taken:

```bash
# Override the port at startup
moltis --port 8080

# Or set it permanently in moltis.toml
# ~/.config/moltis/moltis.toml
[gateway]
port = 8080
```

### Cannot connect from another machine on the network

By default Moltis binds to `127.0.0.1` (localhost only). To allow LAN or remote access:

```bash
moltis --bind 0.0.0.0
```

Then open port 13131 in your firewall:

```bash
# ufw (Ubuntu)
sudo ufw allow 13131/tcp

# firewalld (Fedora/RHEL)
sudo firewall-cmd --add-port=13131/tcp --permanent
sudo firewall-cmd --reload
```

### Sandboxed execution not working

Check that Docker is running and your user can access the socket:

```bash
docker ps                    # Should list running containers (not an error)
groups                       # Check you are in the 'docker' group
sudo usermod -aG docker $USER && newgrp docker
```

### Service fails to start

```bash
journalctl --user -u moltis --no-pager -n 50
```

Look for permission errors on config/data directories:

```bash
ls -la ~/.config/moltis ~/.moltis
```

### Reset authentication

If you are locked out:

```bash
moltis auth reset-password
```

---

## File Locations

| Path | Purpose |
|------|---------|
| `~/.config/moltis/moltis.toml` | Main configuration |
| `~/.config/moltis/credentials.json` | Hashed passwords, passkeys, API tokens |
| `~/.config/moltis/provider_keys.json` | LLM provider API keys |
| `~/.moltis/` | Sessions, databases, memory files, logs |
| `~/.moltis/skills/` | Installed skill repositories |
| `~/.moltis/node.json` | Node connection config (worker nodes only) |
| `~/.local/bin/moltis` | Binary (curl install) |
| `~/.config/systemd/user/moltis.service` | systemd service file |

---

## See Also

- [Quickstart](docs/src/quickstart.md)
- [Configuration reference](docs/src/configuration.md)
- [Cloud Deploy (DigitalOcean, Fly.io, Render)](docs/src/cloud-deploy.md)
- [Multi-Node setup](docs/src/nodes.md)
- [OpenClaw Import](docs/src/openclaw-import.md)
- [Full documentation](https://docs.moltis.org)
