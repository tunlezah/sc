# SoundSync

Bluetooth A2DP audio receiver with multi-room casting to AirPlay and Chromecast devices, 10-band parametric EQ, WebRTC browser streaming, and a real-time web UI. Stream audio from any Bluetooth device to a Linux machine or Raspberry Pi, shape the sound with a built-in equalizer, cast to Chromecast or AirPlay speakers, listen in your browser via WebRTC, and monitor everything from a responsive web interface.

## Features

- **Bluetooth A2DP Sink** — receive audio from phones, tablets, and other devices over Bluetooth
- **A2DP Codec Negotiation** — registers D-Bus MediaEndpoint1 for automatic codec selection (SBC, AAC, LDAC, aptX, aptX HD) with BlueZ
- **Chromecast Audio Output** — cast audio to any Google Cast device (Chromecast Audio, Gen 1/2/3, Ultra, Google Nest/Home) via CASTV2 protocol with mDNS discovery
- **AirPlay Audio Output** — stream to AirPlay receivers using PipeWire's RAOP module with Avahi/mDNS discovery
- **HTTP MP3 Stream** — live MP3 stream at `/api/stream/audio.mp3` (192 kbps CBR) accessible by any media player or Cast device
- **10-Band Parametric EQ** — low-shelf, peaking, and high-shelf filters from 60 Hz to 16 kHz with +/-12 dB gain, processed via PipeWire filter-chain
- **EQ Presets** — 7 built-in presets (Flat, Bass Boost, Vocal, Classical, Rock, Electronic, Podcast) plus custom preset save/load/delete
- **Real-Time Spectrum Visualizer** — 2048-point FFT with 64 logarithmic frequency bands at 48 kHz
- **WebRTC Audio Streaming** — listen to the audio stream directly in your browser with Opus encoding at 128 kbps, full SDP offer/answer and ICE candidate signaling
- **Media Controls** — play, pause, next, previous via AVRCP with track metadata display
- **Line-In Support** — analog audio input source detection and activation
- **REST API & WebSocket** — full programmatic control with real-time event streaming
- **Web UI** — responsive Preact/TypeScript interface with dark mode, Chromecast/AirPlay device management
- **Auto-Pairing** — D-Bus Agent1 that accepts pairing requests automatically
- **Production Ready** — systemd service, install script, CI/CD pipeline

## Supported Devices

### Chromecast (Google Cast)

| Device | Protocol | Status |
|---|---|---|
| Chromecast Audio | CASTV2 | Supported |
| Chromecast Gen 1 | CASTV2 | Supported |
| Chromecast Gen 2 | CASTV2 | Supported |
| Chromecast Gen 3 | CASTV2 | Supported |
| Chromecast Ultra | CASTV2 | Supported |
| Chromecast with Google TV | CASTV2 | Supported |
| Google Nest Mini | CASTV2 | Supported |
| Google Nest Audio | CASTV2 | Supported |
| Google Nest Hub | CASTV2 | Supported |
| Google Home | CASTV2 | Supported |
| Google Home Mini | CASTV2 | Supported |
| Google Home Max | CASTV2 | Supported |

All devices using the Google Cast protocol are supported. The system uses mDNS to discover `_googlecast._tcp.local.` services on the network. Audio is streamed as an HTTP MP3 stream that the Chromecast pulls from SoundSync's HTTP server. This approach supports all Cast protocol versions and device generations.

### AirPlay

| Device | Protocol | Status |
|---|---|---|
| Apple AirPort Express | RAOP | Supported |
| Apple HomePod | RAOP | Supported |
| Apple HomePod mini | RAOP | Supported |
| Apple TV (all generations) | RAOP | Supported |
| Third-party AirPlay speakers | RAOP | Supported |

AirPlay output uses PipeWire's built-in `module-raop-sink` with `module-raop-discover` for native protocol support. Devices are discovered via Avahi mDNS browsing `_raop._tcp`. Audio is routed at the PipeWire graph level using `pw-link`, which means AirPlay output receives EQ-processed audio natively.

## Requirements

- **OS**: Debian, Ubuntu, or Raspberry Pi OS
- **Bluetooth**: adapter (hci0 by default) with BlueZ
- **Audio**: PipeWire with WirePlumber
- **Chromecast**: network connectivity (devices discovered via mDNS)
- **AirPlay**: PipeWire RAOP module + Avahi daemon
- **Build**: Rust stable toolchain, Node.js 22, pkg-config

System packages:

```
bluetooth bluez pipewire pipewire-pulse wireplumber pipewire-module-raop
libdbus-1-dev libpipewire-0.3-dev libspa-0.2-dev libclang-dev libopus-dev libmp3lame-dev pkg-config build-essential
avahi-daemon avahi-utils
```

## Quick Start

### Automated Install (Raspberry Pi / Debian)

```bash
git clone https://github.com/tunlezah/sc.git
cd sc
sudo bash install.sh
sudo systemctl enable --now soundsync
```

The installer handles all dependencies (including Avahi and RAOP modules), builds both backend and frontend, configures Bluetooth and mDNS, creates a systemd service, and installs to `/opt/soundsync`.

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

```
Bluetooth Device (A2DP Source)
        |
    BlueZ Socket
        |
PipeWire Null Sink (soundsync-capture)
        |
PipeWire Monitor Source (soundsync-capture.monitor)
        |
[PCM f32 Audio Broadcast Channel]
    |-- WebRTC Manager --> Opus Encoder --> RTP --> Browser
    |-- Spectrum Analyzer --> FFT --> 64-band spectrum --> WebSocket
    |-- MP3 Encoder --> HTTP Stream --> Chromecast (CASTV2)
    |-- PipeWire Filter-Chain (EQ) --> pw-link --> AirPlay (RAOP Sink)
    +-- Line-In Source (optional input)
```

### Subsystems

1. **BluetoothManager** — manages adapter, device discovery, connections via BlueZ
2. **A2dpEndpoint** — registers `org.bluez.MediaEndpoint1` on D-Bus for each codec, enabling BlueZ to negotiate audio codec parameters with connecting devices
3. **AudioPipeline** — creates a PipeWire null sink, filter-chain (EQ), and audio capture; provides the PCM broadcast channel that all outputs subscribe to
4. **ChromecastManager** — discovers Google Cast devices via mDNS (`_googlecast._tcp.local.`), manages CASTV2 connections via `rust_cast`, instructs Chromecast to play the HTTP MP3 stream URL
5. **AirPlayManager** — discovers AirPlay devices via Avahi (`_raop._tcp`), loads PipeWire RAOP modules, routes audio via `pw-link` from the capture sink to the RAOP sink
6. **WebRtcManager** — accepts browser WebRTC offers, encodes captured PCM audio to Opus via RTP, and streams to browser clients
7. **MP3 Stream Endpoint** — HTTP endpoint (`/api/stream/audio.mp3`) that subscribes to the PCM broadcast, encodes to MP3 192 kbps CBR, and serves as a chunked HTTP response
8. **AvrcpMonitor** — polls BlueZ for playback status and track metadata via AVRCP
9. **SpectrumAnalyzer** — consumes PCM audio and produces 64-band FFT spectrum data
10. **Web Server** — Axum HTTP server with REST API, WebSocket events, and static file serving

### Audio Pipeline Integrity

All audio outputs (WebRTC, Chromecast, AirPlay, spectrum analyzer) hook into the existing PipeWire pipeline at defined extension points:

- **WebRTC & Chromecast**: Subscribe to the PCM broadcast channel (output routing layer)
- **AirPlay**: Routes audio via PipeWire graph links (PipeWire routing layer)
- **Spectrum**: Subscribes to PCM broadcast (analysis layer)

No audio path bypasses the core pipeline. The null sink → monitor source → broadcast channel architecture is preserved exactly as designed.

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
| `GET` | `/api/devices` | List paired/connected Bluetooth devices |
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
| `GET` | `/api/cast/devices` | List discovered Chromecast devices |
| `POST` | `/api/cast/discover` | Scan for Chromecast devices |
| `POST` | `/api/cast/connect` | Connect to a Chromecast |
| `POST` | `/api/cast/disconnect` | Stop casting |
| `POST` | `/api/cast/volume` | Set Chromecast volume |
| `GET` | `/api/airplay/devices` | List discovered AirPlay devices |
| `POST` | `/api/airplay/discover` | Scan for AirPlay devices |
| `POST` | `/api/airplay/connect` | Connect to an AirPlay device |
| `POST` | `/api/airplay/disconnect` | Stop AirPlay |
| `POST` | `/api/airplay/volume` | Set AirPlay volume |
| `GET` | `/api/stream/audio.mp3` | Live MP3 audio stream (192 kbps) |

### WebSocket

Connect to `/ws/status` for real-time events. On connect, receives a full state snapshot including Chromecast and AirPlay device lists and session state. Subsequent events include device state changes, EQ updates, spectrum data, playback status, WebRTC signaling, and Chromecast/AirPlay session events.

**Chromecast Events:** `cast_device_discovered`, `cast_device_removed`, `cast_session_started`, `cast_session_stopped`, `cast_error`

**AirPlay Events:** `air_play_device_discovered`, `air_play_device_removed`, `air_play_session_started`, `air_play_session_stopped`, `air_play_error`

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
│   ├── mp3_encoder.rs   # MP3 encoding for HTTP stream (48kHz stereo 192kbps)
│   ├── webrtc_audio.rs  # WebRTC session manager with RTP streaming
│   ├── chromecast.rs    # ChromecastManager - mDNS discovery, CASTV2 sessions
│   ├── airplay.rs       # AirPlayManager - PipeWire RAOP routing, Avahi discovery
│   ├── cast_stream.rs   # HTTP MP3 stream endpoint handler
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
│   ├── mod.rs           # App state, event bus, snapshots (incl. cast/airplay state)
│   └── config.rs        # Layered TOML configuration
└── web/
    ├── routes.rs        # REST API endpoints (37 endpoints)
    └── ws.rs            # WebSocket handler with WebRTC + Cast/AirPlay events
webui/
├── src/
│   ├── components/      # EQ controls, device list, audio output, spectrum visualizer, etc.
│   ├── hooks/           # App state (with Cast/AirPlay), dark mode
│   └── api/             # REST, WebSocket, WebRTC clients
├── package.json
└── vite.config.ts
scripts/
└── soundsync.service    # Systemd unit file
install.sh               # Automated installer (incl. Avahi, RAOP deps)
```

## Troubleshooting

### Chromecast

- **No devices found**: Ensure your machine and Chromecast are on the same network. Verify mDNS works: `avahi-browse -t -r _googlecast._tcp`
- **Connection fails**: Check firewall rules. SoundSync needs to accept HTTP connections from the Chromecast on port 8080.
- **Audio cuts out**: The Chromecast pulls audio via HTTP. Network interruptions will cause buffering. The system will attempt reconnection automatically.
- **High latency**: MP3 streaming has ~1-3 seconds of inherent latency (encoder buffer + device buffer). This is normal for Cast protocol.

### AirPlay

- **No devices found**: Ensure `avahi-daemon` is running (`systemctl start avahi-daemon`). Verify with `avahi-browse -t -r _raop._tcp`.
- **Cannot connect**: Check if PipeWire RAOP module is installed. Run `find /usr/lib -name "libpipewire-module-raop-sink*"` to verify.
- **Module not loading**: Try manually: `pactl load-module module-raop-discover`. Check PipeWire logs: `journalctl --user -u pipewire`.
- **No audio after connect**: Verify PipeWire links with `pw-link -l`. The system creates links from `soundsync-capture:monitor_FL/FR` to the RAOP sink.

### General Audio

- **No audio pipeline**: Check PipeWire is running: `systemctl --user status pipewire`. Verify with `pw-cli list-objects`.
- **Browser audio not working**: WebRTC requires a secure context (HTTPS) except on localhost. Access via `http://localhost:8080` or set up a reverse proxy with SSL.
- **EQ not applying**: PipeWire filter-chain requires `pipewire-filter-chain` binary. Check it exists: `which pipewire-filter-chain`.

## CI/CD

GitHub Actions runs on every push and PR to `main`:

1. **Rust** — format check, Clippy, build, tests
2. **Frontend** — lint, type check, build
3. **Release** — builds an optimized release binary with the frontend embedded, uploaded as a build artifact

Download the latest release binary from the [Actions tab](../../actions) artifacts.

## Changelog

### v2.0.0

- **Chromecast Audio Output** — full Google Cast support for all Chromecast device types (Audio, Gen 1/2/3, Ultra, Google Nest/Home). Uses mDNS discovery and CASTV2 protocol via `rust_cast`. Audio streams as HTTP MP3 through the existing pipeline.
- **AirPlay Audio Output** — stream to AirPlay receivers using PipeWire's native RAOP module. Avahi-based discovery, `pw-link` routing for EQ-processed audio. Graceful degradation when RAOP module is unavailable.
- **HTTP MP3 Stream Endpoint** — `/api/stream/audio.mp3` serves live 192 kbps CBR MP3 from the PCM broadcast channel. Works with any HTTP media player independently of Chromecast.
- **Web UI Audio Output Panel** — tabbed interface for Chromecast and AirPlay device management with scan, connect, disconnect, and volume controls
- **11 New API Endpoints** — complete REST API for Chromecast and AirPlay device discovery, connection management, and volume control
- **10 New WebSocket Events** — real-time Chromecast and AirPlay device/session state updates
- **Updated Installer** — installs Avahi, RAOP modules, and libmp3lame dependencies; configures mDNS daemon
- **Architecture Preserved** — all new outputs hook into the existing PCM broadcast channel; no pipeline modifications

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
