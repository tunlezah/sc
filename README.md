# SoundSync

Bluetooth A2DP audio receiver with 10-band parametric EQ, WebRTC browser streaming, and a real-time web UI. Stream audio from any Bluetooth device to a Linux machine or Raspberry Pi, shape the sound with a built-in equalizer, listen in your browser via WebRTC, and monitor everything from a responsive web interface.

## Features

- **Bluetooth A2DP Sink** — receive audio from phones, tablets, and other devices over Bluetooth
- **A2DP Codec Negotiation** — registers D-Bus MediaEndpoint1 for automatic codec selection (SBC, AAC, LDAC, aptX, aptX HD) with BlueZ
- **10-Band Parametric EQ** — low-shelf, peaking, and high-shelf filters from 60 Hz to 16 kHz with +/-12 dB gain, processed via PipeWire filter-chain
- **EQ Presets** — 7 built-in presets (Flat, Bass Boost, Vocal, Classical, Rock, Electronic, Podcast) plus custom preset save/load/delete
- **Real-Time Spectrum Visualizer** — 2048-point FFT with 64 logarithmic frequency bands at 48 kHz
- **WebRTC Audio Streaming** — listen to the audio stream directly in your browser with Opus encoding at 128 kbps, full SDP offer/answer and ICE candidate signaling
- **Media Controls** — play, pause, next, previous via AVRCP with track metadata display
- **Line-In Support** — analog audio input source detection and activation
- **REST API & WebSocket** — full programmatic control with real-time event streaming
- **Web UI** — responsive Preact/TypeScript interface with dark mode
- **Auto-Pairing** — D-Bus Agent1 that accepts pairing requests automatically
- **Production Ready** — systemd service, install script, CI/CD pipeline

## Requirements

- **OS**: Debian, Ubuntu, or Raspberry Pi OS
- **Bluetooth**: adapter (hci0 by default) with BlueZ
- **Audio**: PipeWire with WirePlumber
- **Build**: Rust stable toolchain, Node.js 22, pkg-config

System packages:

```
bluetooth bluez pipewire pipewire-pulse wireplumber
libdbus-1-dev libpipewire-0.3-dev libspa-0.2-dev libclang-dev libopus-dev pkg-config build-essential
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

## Architecture

SoundSync runs as a single process with several concurrent subsystems:

1. **BluetoothManager** — manages adapter, device discovery, connections via BlueZ
2. **A2dpEndpoint** — registers `org.bluez.MediaEndpoint1` on D-Bus for each codec, enabling BlueZ to negotiate audio codec parameters with connecting devices
3. **AudioPipeline** — creates a PipeWire null sink, filter-chain (EQ), and audio capture
4. **WebRtcManager** — accepts browser WebRTC offers, encodes captured PCM audio to Opus via RTP, and streams to browser clients
5. **AvrcpMonitor** — polls BlueZ for playback status and track metadata via AVRCP
6. **SpectrumAnalyzer** — consumes PCM audio and produces 64-band FFT spectrum data
7. **Web Server** — Axum HTTP server with REST API, WebSocket events, and static file serving

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

Connect to `/ws/status` for real-time events. On connect, receives a full state snapshot. Subsequent events include device state changes, EQ updates, spectrum data, playback status, and WebRTC signaling.

**WebRTC Signaling** is handled over the same WebSocket connection:
- Client sends `webrtc_offer` with SDP
- Server responds with `webrtc_answer` and `webrtc_ice_candidate` events
- Client sends `webrtc_ice_candidate` for trickle ICE
- Client sends `webrtc_stop` to end the session

## Project Structure

```
src/
├── main.rs              # Entry point, wires all subsystems
├── audio/
│   ├── pipeline.rs      # PipeWire null sink + filter-chain management
│   ├── capture.rs       # PCM audio capture via pw-cat/parec
│   ├── opus_encoder.rs  # Opus encoding for WebRTC (48kHz stereo 128kbps)
│   ├── webrtc_audio.rs  # WebRTC session manager with RTP streaming
│   ├── spectrum.rs      # FFT spectrum analyzer (64 bands)
│   ├── filter_chain.rs  # PipeWire filter-chain config generation
│   └── line_in.rs       # Line-in source detection
├── bluetooth/
│   ├── manager.rs       # BlueZ adapter, discovery, connections
│   ├── endpoint.rs      # A2DP MediaEndpoint1 D-Bus implementation
│   ├── agent.rs         # Pairing agent (auto-accept)
│   ├── avrcp.rs         # AVRCP media controls and track info
│   ├── discovery.rs     # Device discovery and state management
│   ├── device.rs        # Device info and state machine
│   ├── constants.rs     # UUIDs, paths, timing constants
│   └── codecs/          # SBC, AAC, LDAC, aptX, aptX HD capability negotiation
├── dsp/
│   ├── equalizer.rs     # 10-band parametric EQ with PipeWire filter-chain
│   ├── biquad.rs        # Biquad filter coefficient calculation
│   └── presets.rs       # Built-in and custom EQ presets
├── state/
│   ├── mod.rs           # App state, event bus, snapshots
│   └── config.rs        # Layered TOML configuration
└── web/
    ├── routes.rs        # REST API endpoints
    └── ws.rs            # WebSocket handler with WebRTC signaling
webui/
├── src/
│   ├── components/      # EQ controls, device list, spectrum visualizer, audio player, etc.
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

## Changelog

### v1.1.0

- **A2DP Codec Negotiation** — added `MediaEndpoint1` D-Bus endpoint registration for all 5 codecs, enabling proper A2DP sink functionality with automatic codec selection
- **Working WebRTC Audio** — full pipeline from PipeWire capture through Opus encoding to RTP streaming; browser clients can now listen to audio in real-time
- **WebRTC Signaling** — complete SDP offer/answer and ICE candidate exchange over WebSocket with per-session routing
- **Bug fixes:**
  - Fixed spectrum analyzer sample rate (was 44100 Hz, now correctly 48000 Hz to match capture)
  - Fixed Bluetooth status string format (REST API now returns serde-compatible snake_case values)
  - Fixed AudioPlayer WebSocket handler leak (handlers now properly unsubscribed on stop)
  - Removed unused `clap` and `thiserror` dependencies

### v1.0.0

- Initial release

## License

MIT
