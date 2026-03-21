# StreamCast vs sc (SoundSync) - Full Code-Level Comparison

Both repos are **Rust + Preact/TypeScript** projects implementing a **Bluetooth A2DP audio receiver** with DSP equalizer and web UI, targeting Linux/Raspberry Pi. They share the same conceptual DNA but diverge significantly in architecture, maturity, and completeness.

## Project Structure

| Aspect | **StreamCast** | **sc** |
|--------|---------------|--------|
| Rust LOC | 4,517 | 4,291 |
| Files with `#[cfg(test)]` | 9 | 17 |
| Extra Rust files | `events.rs`, `config.rs`, `endpoint.rs`, `opus_encoder.rs` | `state/config.rs` (module split) |
| Extra project files | None | `install.sh`, `scripts/soundsync.service`, `README.md`, `.gitignore`, `Cargo.lock`, `.githooks/pre-commit` |

## Dependencies & Build

| Dep | **StreamCast** | **sc** |
|-----|---------------|--------|
| `tower-http` | 0.5 | **0.6** (newer) |
| `dirs` | 5 | **6** (newer) |
| `thiserror` | absent | **2** (proper error types) |
| `libc` | absent | **0.2** (XDG_RUNTIME_DIR) |
| `rtp` + `bytes` | explicit deps | absent (WebRTC less complete) |
| `dev-dependencies` | **none** | `tokio-test`, `axum-test` |

## State Management

**StreamCast** splits state into 3 files (`state.rs`, `events.rs`, `config.rs`):
- `AppStateHandle` holds `bt_cmd_tx` and `webrtc_cmd_tx` channels
- `Config::load()` uses `PartialConfig` for true field-level merging
- `BluetoothStatus` has 5 variants including `Initializing` and `Discoverable`

**sc** consolidates into `state/mod.rs` + `state/config.rs`:
- Leaner `AppStateHandle` (no command channels - those live in `AppRouter`)
- `AppStateSnapshot` struct + `snapshot()` method for clean serialization
- `SystemEvent` is richer (`StreamStarted{codec}`, `Error{message}`, `ServiceStopping`)
- **4 unit tests** for state (playback parsing, snapshot, pubsub)

## Bluetooth Manager

**StreamCast** (237 lines):
- `BluetoothManager::new()` is async, creates BlueZ session up front
- Has **`A2dpEndpoint`** (88 lines) - D-Bus `MediaEndpoint1` for codec negotiation (CRITICAL for A2DP sink functionality)
- Simple command loop, discovery spawned as separate task

**sc** (332 lines):
- `run()` does initialization inline with graceful error handling
- Main loop uses `select!` on 3 branches: commands, discovery stream, AND device property polling
- Has `SetName` command, `poll_device_properties()` for state reconciliation
- **No A2dpEndpoint** - cannot negotiate codecs with BlueZ

## Audio Pipeline & WebRTC

**StreamCast**:
- `OpusEncoder` (39 lines) - wraps opus crate at 48kHz stereo 128kbps
- `WebRtcManager` (345 lines) - full session management with audio capture, RTP packet construction, proper lifecycle
- **Actually streams audio** to browser via WebRTC

**sc**:
- `WebRtcManager` (171 lines) - creates peer connections but **does not pump audio**
- No `OpusEncoder` - missing the entire audio encoding path
- WebRTC signaling is stubbed (just logs, doesn't act)

## DSP / Equalizer

**StreamCast**: Full `Equalizer` struct with `StereoBiquad` filters, `process_stereo()` runs audio through filter chain in-memory.

**sc**: Generates **PipeWire filter-chain configuration** strings, delegating actual DSP to PipeWire's native module. Better for production (lower latency, native audio graph integration).

## Web API

**StreamCast** (390 lines): Uses raw `Json<Value>` request bodies, `state.send_bt_command()` helper.

**sc** (311 lines): Uses **typed request structs** (`ScanRequest`, `AddressRequest`, `EqRequest`), typed `OkResponse`, dedicated AVRCP command channel, preset deletion endpoint.

## WebSocket Layer

**StreamCast** (193 lines): Sends `SystemEvent` directly as JSON (coupled). Full WebRTC signaling forwarding.

**sc** (265 lines): Dedicated `WsOutMessage`/`WsInMessage` enums (decoupled internal events from wire format). Handles `broadcast::RecvError::Lagged`. WebRTC signaling is stubbed.

## Testing

| Area | StreamCast | sc |
|------|-----------|-----|
| Test files | 9 | **17** |
| State/Config | 0 | 7 |
| Codecs | 10 | 12+ (mod + per-codec) |
| Presets | 6 | 12 |
| Dev deps | none | `tokio-test`, `axum-test` |

## CI / Deployment

**StreamCast**: No install script, no systemd, no GitHub Releases.

**sc**: Full `install.sh` (257 lines), systemd service, GitHub Releases on tags, `.githooks/pre-commit`.

## Summary Scorecard

| Dimension | Winner | Notes |
|-----------|--------|-------|
| Config system | StreamCast | True layered merging via `PartialConfig` |
| A2DP endpoint | **StreamCast** | Critical for Bluetooth audio sink |
| WebRTC audio | **StreamCast** | Full pipeline vs stub |
| DSP approach | **sc** | PipeWire-native is production-ready |
| API design | **sc** | Typed request/response structs |
| Test coverage | **sc** | 17 test files vs 9 |
| Error handling | **sc** | `thiserror`, typed errors |
| Deployment | **sc** | Install script, systemd, releases |
| BT robustness | **sc** | Property polling, graceful degradation |
| Dependencies | **sc** | Newer versions, dev-deps |

## Recommendation

**sc is the better codebase to move forward with** due to superior test coverage, production deployment readiness, better Rust practices, and PipeWire-native DSP.

**Port these from StreamCast to sc:**
1. **`A2dpEndpoint`** (`endpoint.rs`) - D-Bus `MediaEndpoint1` for codec negotiation
2. **WebRTC audio pipeline** - `OpusEncoder`, `AudioCapture` subscription, RTP packet construction
3. **Layered config merging** - `PartialConfig` approach from `config.rs`
