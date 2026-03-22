# AirPlay & Chromecast Audio Output for SoundSync

## Context

SoundSync is a Rust Bluetooth A2DP receiver that processes audio server-side (PipeWire EQ/DSP) and streams to browsers via WebRTC/Opus. The user wants to add AirPlay and Chromecast as audio output targets, preferring **server-side streaming** (no browser involvement). Research confirms this is feasible without SSL/certificates for both protocols.

**Key finding:** Chromecast can pull media from plain HTTP URLs (no SSL needed). AirPlay can be handled natively by PipeWire's built-in RAOP module. Both are fully server-side.

---

## Architecture Decision

**Chromecast:** Server subscribes to the existing PCM broadcast channel, encodes to MP3, serves an HTTP stream endpoint. Uses `rust_cast` to tell the Chromecast to play that URL. No browser needed, no SSL needed.

**AirPlay:** Leverages PipeWire's built-in `module-raop-discover`/`module-raop-sink` to route audio at the PipeWire graph level. No Rust AirPlay protocol implementation needed. Uses subprocess commands (`pw-link`, `pactl`) matching the project's existing PipeWire integration pattern.

---

## Implementation Plan

### Phase 1: Chromecast Support

#### 1.1 New Dependencies (`Cargo.toml`)
```toml
rust_cast = "0.19"       # CASTV2 protocol
mdns-sd = "0.11"         # mDNS device discovery
mp3lame-encoder = "0.2"  # PCM→MP3 for HTTP stream
```

#### 1.2 New Files

| File | Purpose |
|------|---------|
| `src/audio/chromecast.rs` | ChromecastManager - mDNS discovery, rust_cast connection, session lifecycle |
| `src/audio/cast_stream.rs` | Axum handler for `GET /api/stream/audio.mp3` - subscribes to broadcast channel, encodes MP3, streams chunked HTTP |
| `src/audio/mp3_encoder.rs` | MP3 encoder wrapper (follows pattern of `src/audio/opus_encoder.rs`) |

#### 1.3 ChromecastManager Design
- Follows `WebRtcManager` pattern (`src/audio/webrtc_audio.rs`): command enum + mpsc channel + `run()` loop
- Commands: `Discover`, `Connect { device_id }`, `Disconnect`, `SetVolume { level }`
- Discovery: browse `_googlecast._tcp.local.` via `mdns-sd::ServiceDaemon`
- Connection: `rust_cast::CastDevice` → load media URL `http://<server-ip>:<port>/api/stream/audio.mp3`
- Server IP detection: inspect local address of TCP connection to Chromecast

#### 1.4 HTTP Stream Endpoint (`/api/stream/audio.mp3`)
- Subscribes to `pipeline.audio_sender()` broadcast channel
- Encodes PCM f32 48kHz stereo → MP3 192kbps CBR
- Returns `axum::body::Body::from_stream()` with `Content-Type: audio/mpeg`
- Independently useful: any media player can open this URL directly

### Phase 2: AirPlay Support

#### 2.1 No New Crate Dependencies
Uses PipeWire/Avahi via subprocess (matching existing pattern at `Cargo.toml:19`)

#### 2.2 New File

| File | Purpose |
|------|---------|
| `src/audio/airplay.rs` | AirPlayManager - PipeWire RAOP discovery and routing |

#### 2.3 AirPlayManager Design
- Same command/manager pattern as ChromecastManager
- Commands: `Discover`, `Connect { name }`, `Disconnect`, `SetVolume { level }`
- Discovery: `avahi-browse -t -r _raop._tcp` or `pw-cli list-objects` after loading RAOP discover module
- Connect: `pactl load-module module-raop-discover` then `pw-link soundsync-capture:monitor_FL <raop-sink>:playback_FL` (and FR)
- Disconnect: `pw-link -d` to unlink nodes
- **Advantage:** AirPlay output gets EQ-processed audio natively through PipeWire graph (no broadcast channel subscription needed)

#### 2.4 Prerequisites Check
- `AirPlayManager::new()` checks for `pipewire-module-raop-sink` and Avahi daemon
- Logs warnings but doesn't fail (matches graceful degradation pattern at `main.rs:64-67`)

### Phase 3: State & API Changes

#### 3.1 State (`src/state/mod.rs`)
Add to `SystemEvent` enum:
- `CastDeviceDiscovered`, `CastDeviceRemoved`, `CastSessionStarted`, `CastSessionStopped`, `CastError`
- `AirPlayDeviceDiscovered`, `AirPlayDeviceRemoved`, `AirPlaySessionStarted`, `AirPlaySessionStopped`, `AirPlayError`

Add structs: `CastDeviceInfo { id, name, address, port }`, `AirPlayDeviceInfo { name, address, port }`

Add to `AppState`: `cast_devices`, `cast_active`, `airplay_devices`, `airplay_active`

#### 3.2 API Endpoints (`src/web/routes.rs`)
- `GET /api/cast/devices` - list Chromecast devices
- `POST /api/cast/discover` - trigger scan
- `POST /api/cast/connect` - connect by device_id
- `POST /api/cast/disconnect` - stop casting
- `POST /api/cast/volume` - set volume
- `GET /api/airplay/devices` - list AirPlay receivers
- `POST /api/airplay/discover` - trigger scan
- `POST /api/airplay/connect` - connect by name
- `POST /api/airplay/disconnect` - stop AirPlay
- `POST /api/airplay/volume` - set volume
- `GET /api/stream/audio.mp3` - HTTP audio stream

#### 3.3 WebSocket (`src/web/ws.rs`)
Add handlers for new event types → push device/session updates to frontend

### Phase 4: Main.rs Wiring (`src/main.rs`)
Following the WebRTC pattern (lines 76-88):
1. Create `mpsc::channel::<ChromecastCommand>(32)` and `mpsc::channel::<AirPlayCommand>(32)`
2. Subscribe to `pipeline.audio_sender()` for Chromecast
3. Spawn both managers as tokio tasks
4. Pass command senders into `AppRouter`

### Phase 5: Frontend (`webui/`)

#### New Component: `webui/src/components/AudioOutput/AudioOutput.tsx`
- Section with Chromecast and AirPlay tabs
- Scan button, device list, connect/disconnect, volume slider, status indicator

#### Modified Files:
- `webui/src/types.ts` - add `CastDevice`, `AirPlayDevice` interfaces, extend `AppState`
- `webui/src/api/rest.ts` - add API functions for all new endpoints
- `webui/src/hooks/useAppState.ts` - handle new WebSocket message types
- `webui/src/app.tsx` - add `AudioOutput` component

---

## Files to Modify (Summary)

**New files (5):**
- `src/audio/chromecast.rs`
- `src/audio/cast_stream.rs`
- `src/audio/mp3_encoder.rs`
- `src/audio/airplay.rs`
- `webui/src/components/AudioOutput/AudioOutput.tsx`

**Modified files (10):**
- `Cargo.toml` - new dependencies
- `src/audio/mod.rs` - declare new modules
- `src/state/mod.rs` - new events, structs, state fields
- `src/web/routes.rs` - new endpoints, AppRouter fields
- `src/web/ws.rs` - new event handlers
- `src/main.rs` - spawn managers, wire channels
- `webui/src/types.ts` - new interfaces
- `webui/src/api/rest.ts` - new API functions
- `webui/src/hooks/useAppState.ts` - new WS handlers
- `webui/src/app.tsx` - add component

---

## Implementation Order

1. **Chromecast first** - fully server-side, well-documented protocol, `rust_cast` handles heavy lifting
2. **HTTP stream endpoint** - independently valuable (works in any media player)
3. **AirPlay second** - depends on PipeWire RAOP module availability
4. **Frontend last** - backend testable via curl/CLI first

---

## Verification

1. **Chromecast:** Build and run. Use `curl http://localhost:8080/api/stream/audio.mp3 | mpv -` to verify the MP3 stream works. Then `POST /api/cast/discover` and `POST /api/cast/connect` to test with a real Chromecast.
2. **AirPlay:** Run `avahi-browse -t -r _raop._tcp` to verify AirPlay devices are discoverable. Then test via API endpoints.
3. **Frontend:** Open the web UI, verify device lists populate, connect/disconnect works, volume slider functions.
4. **Concurrent output:** Verify Chromecast + WebRTC + spectrum analyzer all work simultaneously (broadcast channel supports multiple subscribers).

---

## Potential Challenges

- **Server IP detection** for Chromecast URL: inspect local socket address from the TCP connection to the Chromecast
- **MP3 latency:** expect 1-3 seconds on Chromecast (encoder buffer + device buffer); use CBR and flush after each frame
- **PipeWire RAOP availability:** not all distros include it; graceful degradation if missing
- **Chromecast heartbeat:** `rust_cast` handles CASTV2 keepalives but connection must be maintained in background task
