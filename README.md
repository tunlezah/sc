# SoundSync

Bluetooth A2DP audio receiver with 10-band parametric EQ and a real-time web UI. Stream audio from any Bluetooth device to a Linux machine or Raspberry Pi, shape the sound with a built-in equalizer, and monitor everything from your browser.

## Features

- **Bluetooth A2DP Sink** — receive audio from phones, tablets, and other devices over Bluetooth (SBC, AAC, LDAC, aptX, aptX HD)
- **10-Band Parametric EQ** — low-shelf, peaking, and high-shelf filters from 60 Hz to 16 kHz with ±12 dB gain and built-in presets (Flat, Bass Boost, Vocal, Classical, Rock, Electronic, Podcast)
- **Real-Time Spectrum Visualizer** — FFT-based frequency analysis displayed in the browser
- **Media Controls** — play, pause, next, previous via AVRCP
- **Line-In Support** — analog audio input source
- **WebRTC Audio Streaming** — listen to the audio stream directly in your browser
- **REST API & WebSocket** — full programmatic control with real-time event streaming
- **Web UI** — responsive Preact/TypeScript interface with dark mode

## Requirements

- **OS**: Debian, Ubuntu, or Raspberry Pi OS
- **Bluetooth**: adapter (hci0 by default) with BlueZ
- **Audio**: PipeWire with WirePlumber
- **Build**: Rust stable toolchain, Node.js 22, pkg-config

System packages:

```
bluetooth bluez pipewire pipewire-pulse wireplumber
libdbus-1-dev libpipewire-0.3-dev libspa-0.2-dev libclang-dev pkg-config build-essential
```

## Quick Start

### Automated Install (Raspberry Pi / Debian)

```bash
git clone https://github.com/tunlezah/sc.git
cd sc
sudo bash install.sh
sudo systemctl enable --now soundsync
```

The installer handles all dependencies, builds both backend and frontend, configures Bluetooth, creates a systemd service, and installs to `/opt/soundsync`.

### Manual Build

```bash
# Backend
cargo build --release

# Frontend
cd webui
npm ci
npm run build
cd ..

# Run
./target/release/soundsync
```

The web UI is served at `http://localhost:8080` by default.

## Configuration

SoundSync loads configuration from these locations (last wins):

1. `/etc/soundsync/config.toml`
2. `~/.config/soundsync/config.toml`
3. `./config.toml`

```toml
port = 8080              # HTTP server port
adapter = "hci0"         # Bluetooth adapter
device_name = "SoundSync" # Bluetooth display name
auto_pair = true         # Auto-accept pairing requests
max_devices = 1          # Max concurrent connections
```

### Environment Variables

| Variable | Default | Description |
|---|---|---|
| `RUST_LOG` | `info` | Log level (`debug`, `info`, `warn`, `error`) |
| `XDG_RUNTIME_DIR` | auto | Required by PipeWire (set automatically) |

## API

### REST Endpoints

| Method | Path | Description |
|---|---|---|
| `GET` | `/api/status` | Server status and uptime |
| `GET` | `/api/devices` | List paired/connected devices |
| `POST` | `/api/bluetooth/scan` | Start/stop device discovery |
| `POST` | `/api/bluetooth/connect` | Connect to a device |
| `POST` | `/api/bluetooth/disconnect` | Disconnect a device |
| `DELETE` | `/api/bluetooth/device` | Remove a paired device |
| `POST` | `/api/bluetooth/name` | Set Bluetooth display name |
| `GET` | `/api/eq` | Current EQ settings |
| `POST` | `/api/eq` | Update EQ bands |
| `GET` | `/api/eq/presets` | List EQ presets |
| `POST` | `/api/eq/preset` | Apply a preset |
| `POST` | `/api/eq/preset/save` | Save a custom preset |
| `DELETE` | `/api/eq/preset/{name}` | Delete a preset |
| `GET` | `/api/line-in/status` | Line-in availability |
| `POST` | `/api/line-in/activate` | Activate line-in |
| `POST` | `/api/line-in/deactivate` | Deactivate line-in |
| `POST` | `/api/avrcp/play` | Play |
| `POST` | `/api/avrcp/pause` | Pause |
| `POST` | `/api/avrcp/next` | Next track |
| `POST` | `/api/avrcp/previous` | Previous track |

### WebSocket

Connect to `/ws/status` for real-time events (device changes, EQ updates, spectrum data, playback status).

## Project Structure

```
src/
├── main.rs              # Entry point
├── audio/               # PipeWire pipeline, capture, spectrum analysis, WebRTC
├── bluetooth/           # BlueZ integration, discovery, pairing, AVRCP, codecs
├── dsp/                 # Biquad filters, parametric EQ, presets
├── state/               # App state, config loading, event bus
└── web/                 # Axum routes, WebSocket handler
webui/
├── src/
│   ├── components/      # EQ controls, device list, spectrum visualizer, etc.
│   ├── hooks/           # App state, dark mode
│   └── api/             # REST, WebSocket, WebRTC clients
├── package.json
└── vite.config.ts
scripts/
└── soundsync.service    # Systemd unit file
install.sh               # Automated installer
```

## CI/CD

GitHub Actions runs on every push and PR to `main`:

1. **Rust** — format check, Clippy, build, tests
2. **Frontend** — lint, type check, build
3. **Release** — builds an optimized release binary with the frontend embedded, uploaded as a build artifact

Download the latest release binary from the [Actions tab](../../actions) artifacts.

## License

MIT
