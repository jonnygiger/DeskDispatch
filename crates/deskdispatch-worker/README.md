# DeskDispatch Worker PC Client (Debian Linux)

`deskdispatch-worker` is a lightweight, high-performance GUI task execution client built for standard x86 Debian Linux machines. It runs as a background service or standalone CLI, long-polling the DeskDispatch task server for automation assignments, capturing screenshots, searching template bitmaps, and simulating mouse and keyboard actions.

---

## Features & Capabilities

- **X11 Desktop Automation:** Captures screen display buffers and executes mouse clicks (`left`, `right`, `middle`, `single`, `double`) and keypress chords (`ctrl+c`, `tab`, `enter`, `F5`, etc.).
- **Sub-Template Bitmap Search:** Performs template bitmap searches across screen regions with configurable match thresholds (`0.0` to `1.0`).
- **Flexible Worker Registration:**
  - **WPS Unattended Mode:** Registers automatically using a static server pre-shared key (`DESKDISPATCH_PSK`).
  - **Token Registration:** Exchanges single-use registration tokens (`DESKDISPATCH_REGISTRATION_TOKEN`) for permanent API keys.
- **Systemd Management:** Embedded CLI subcommands to generate, install, enable, status-check, and uninstall systemd unit files on Debian Linux.

---

## Deployment & Installation Guide

### Prerequisites (Debian / Ubuntu)

Ensure X11 desktop environment and automation utilities are installed:

```bash
sudo apt-get update
sudo apt-get install -y xdotool scrot imagemagick
```

### Quick Binary Build

Build the release binary across the workspace:

```bash
cargo build --release -p deskdispatch-worker
```

The resulting binary will be at `target/release/deskdispatch-worker`.

---

## Service Operations & Management Commands

### 1. Generate Configuration File

Create default configuration file at `/etc/deskdispatch-worker/config.json`:

```bash
sudo ./target/release/deskdispatch-worker config-gen --output /etc/deskdispatch-worker/config.json
```

**Sample `config.json`:**
```json
{
  "server_url": "http://192.168.1.10:3000",
  "psk": "deskdispatch_wps_psk_prod_secret",
  "registration_token": null,
  "api_key": null,
  "hostname": "warehouse-pc-01",
  "display_name": "Warehouse PC 01",
  "display": ":0",
  "log_level": "info",
  "screen_width": 1920,
  "screen_height": 1080
}
```

### 2. Install and Start Systemd Service

Install `deskdispatch-worker` as a background systemd service:

```bash
sudo ./target/release/deskdispatch-worker service install --user root
```

This creates `/etc/systemd/system/deskdispatch-worker.service`, reloads systemd, enables autostart on system boot, and starts the worker process.

### 3. Check Service Status

Query systemd service runtime status:

```bash
sudo ./target/release/deskdispatch-worker service status
```

Or view live system logs:

```bash
journalctl -u deskdispatch-worker -f
```

### 4. Uninstall Systemd Service

Disable and remove the service unit:

```bash
sudo ./target/release/deskdispatch-worker service uninstall
```

---

## Environment Variable Configuration

All settings can be overridden via environment variables or CLI arguments:

| Variable | CLI Argument | Default | Description |
| --- | --- | --- | --- |
| `DESKDISPATCH_SERVER_URL` | `-s`, `--server-url` | `http://localhost:3000` | DeskDispatch Task Server URL |
| `DESKDISPATCH_PSK` | `--psk` | `None` | Pre-Shared Key for WPS registration |
| `DESKDISPATCH_REGISTRATION_TOKEN` | `-t`, `--token` | `None` | One-time worker registration token |
| `DESKDISPATCH_API_KEY` | `-k`, `--api-key` | `None` | Saved machine API key |
| `WORKER_HOSTNAME` | `--hostname` | System hostname | Worker node network hostname |
| `WORKER_DISPLAY_NAME` | `--display-name` | Worker Node (...) | Operator UI display name |
| `DISPLAY` | `--display` | `:0` | Target X11 DISPLAY target |
| `RUST_LOG` | `--log-level` | `info` | Logging verbosity (`info`, `debug`, `trace`) |

---

## Standalone Execution

Run directly in foreground mode for debugging or manual testing:

```bash
./target/release/deskdispatch-worker --server-url http://192.168.1.10:3000 --psk deskdispatch_wps_psk_secret run
```
