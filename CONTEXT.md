# CONTEXT.md — AI Agent Prompt Context for SoundSync

> Feed this file at the start of every AI coding session.
> Last updated: 2026-03-28 | Version: 2.5.0

---

## 1. Project Overview

**SoundSync** is a Bluetooth A2DP audio receiver for Linux/Raspberry Pi that streams to Chromecast, AirPlay devices, and browsers via WebRTC, with a 10-band parametric equalizer and responsive web UI.

- **Repo:** https://github.com/tunlezah/sc
- **Version:** 2.5.0
- **License:** MIT
- **Backend:** Rust (Axum, Tokio, bluer, PipeWire)
- **Frontend:** TypeScript/Preact + Vite
- **Target:** Raspberry Pi / Linux with PipeWire + WirePlumber

---

## 2. Architecture (9 Subsystems)

```
Phone ──BT A2DP──► [BlueZ + WirePlumber] ──► [PipeWire Null Sink]
                                                      │
                                    ┌─────────────────┼─────────────────┐
                                    ▼                 ▼                 ▼
                              [EQ Filter]      [Spectrum FFT]    [Raw PCM Bus]
                                    │                                  │
                              ┌─────┼─────┐                    ┌──────┼──────┐
                              ▼     ▼     ▼                    ▼      ▼      ▼
                          WebRTC  Chromecast  AirPlay      REST API  WS   Web UI
```

**Key principle:** WirePlumber owns ALL Bluetooth codec negotiation and transport. We do NOT register custom A2DP endpoints.

---

## 3. Tech Stack

| Layer | Tech | Key Crates/Packages |
|-------|------|---------------------|
| Bluetooth | bluer 0.17, zbus 4 | D-Bus agent, AVRCP polling |
| Audio | PipeWire, parec/pw-cat | Null sink, filter-chain EQ |
| Codecs | opus 0.3, mp3lame 0.2, ffmpeg (AAC) | Opus→WebRTC, AAC→Chromecast |
| Casting | rust_cast 0.19, mdns-sd 0.11 | CASTV2, RAOP, mDNS |
| Web | axum 0.7, tokio 1, tower-http 0.6 | 37 REST endpoints, WebSocket |
| WebRTC | webrtc 0.12 | SDP signaling, RTP |
| Frontend | Preact 10.25, Vite 6, TypeScript 5.6 | Single-page app |

---

## 4. Directory Structure (Key Files)

```
src/
├── main.rs                    # Entry point, subsystem orchestration
├── audio/
│   ├── pipeline.rs            # PipeWire null sink, filter-chain
│   ├── capture.rs             # PCM capture (parec/pw-cat)
│   ├── chromecast.rs          # Google Cast CASTV2
│   ├── airplay.rs             # AirPlay RAOP routing
│   ├── webrtc_audio.rs        # WebRTC sessions
│   ├── aac_encoder.rs         # AAC-LC 256kbps
│   ├── opus_encoder.rs        # Opus → RTP
│   └── spectrum.rs            # 2048-pt FFT → 64 bands
├── bluetooth/
│   ├── manager.rs             # BlueZ adapter, discovery, state
│   ├── agent.rs               # Auto-pairing D-Bus agent
│   ├── avrcp.rs               # Media controls & metadata
│   ├── device.rs              # DeviceState machine
│   └── endpoint.rs            # DISABLED — reference only
├── dsp/
│   ├── equalizer.rs           # 10-band parametric EQ
│   └── presets.rs             # 7 built-in + custom presets
├── state/
│   ├── mod.rs                 # SystemEvent, AppState, AppStateHandle
│   └── config.rs              # Layered TOML config
└── web/
    ├── routes.rs              # 37 REST endpoints
    └── ws.rs                  # WebSocket events
webui/
├── src/
│   ├── app.tsx, main.tsx, version.ts, types.ts
│   ├── components/            # Header, DeviceList, EQ, Spectrum, etc.
│   ├── hooks/                 # useAppState, useDarkMode
│   └── api/                   # rest.ts, websocket.ts, webrtc.ts
├── package.json
└── dist/                      # Built production UI
install.sh                     # Automated deployment (257 lines)
scripts/soundsync.service      # systemd unit
```

---

## 5. Current Status

- **Stable at v2.5.0** — Bluetooth A2DP, Chromecast, AirPlay, WebRTC, EQ all functional
- **88+ commits, 40 PRs merged**
- **CI passing** — GitHub Actions (Rust fmt/clippy/test + frontend lint)
- **Recent work:** Audio pipeline stabilization, WirePlumber integration, installer hardening

---

## 6. Known Recurring Issues (Learn From History)

These patterns have caused repeated bugs across multiple PRs. **Check these EVERY time you make changes.**

### 6.1 Code That Doesn't Compile (MOST COMMON)

**Pattern:** Code pushed that fails `cargo fmt`, `cargo clippy`, or `cargo build`. This has happened in 10+ commits.

**Specific recurring lint failures:**
- `manual_map` — use `.map()` instead of match/if-let returning Some/None
- `into_iter_on_ref` — use `.iter()` not `.into_iter()` on borrowed collections
- `large_enum_variant` — Box large variants in enums
- `items_after_test_module` — nothing after `#[cfg(test)] mod tests`
- `enum_variant_names` — don't prefix variants with the enum name

**Rule:** ALWAYS run the full pre-commit checklist before committing:
```bash
cargo fmt --all --check && cargo clippy --all-targets --all-features -- -D warnings && cargo test && cargo build
```

### 6.2 Audio Pipeline Breaks (HIGHEST IMPACT)

**Pattern:** ~50% of recent PRs were audio pipeline fixes. The pipeline is fragile because it spans PipeWire, WirePlumber, BlueZ, parec, and filter-chain — any misconfiguration causes silent failure (no audio, no error).

**Things that have broken it:**
- Null sink created but not set as default sink
- parec using wrong parameters or wrong source name
- Filter-chain missing but pipeline doesn't error
- PipeWire node names vs PulseAudio names confused
- Timing issues — null sink not ready when capture starts (need retry)
- Capture using wrong 3-tier source resolution order

**Rule:** After ANY audio pipeline change, mentally trace the full signal path: BT source → null sink → EQ filter → capture → encode → output.

### 6.3 Missing System Dependencies

**Pattern:** New features added but `install.sh` not updated with required packages.

**Packages that were missed and caused production failures:**
- `libspa-0.2-bluetooth` — Without this, WirePlumber cannot see Bluetooth AT ALL
- `pulseaudio-utils` — Provides `pactl`/`parec`, separate from `pipewire-pulse`
- `ffmpeg` — Required for AAC encoding

**Rule:** When adding ANY new system dependency, update `install.sh` immediately.

### 6.4 Version Number Desync

**Pattern:** Version updated in one file but not all three locations. Once, `version.ts` was stuck at v2.3.0 while everything else was v2.5.0.

**All three files that must be updated together:**
| File | Location |
|------|----------|
| `Cargo.toml` | `version = "X.Y.Z"` (line ~3) |
| `webui/package.json` | `"version": "X.Y.Z"` (line ~4) |
| `install.sh` | `VERSION="X.Y.Z"` (line ~15) |

Also update `webui/src/version.ts` if it exists with a hardcoded version.

**WARNING:** Never use `$(cargo pkgid)` or shell commands to derive versions.

### 6.5 WirePlumber/A2DP Endpoint Conflicts

**Pattern:** Registering custom MediaEndpoint1 with BlueZ steals transports from WirePlumber. This caused 3+ PRs of debugging "no audio" issues.

**Rule:** `src/bluetooth/endpoint.rs` is REFERENCE ONLY. Never enable custom A2DP endpoint registration. WirePlumber must own all Bluetooth transports.

### 6.6 Service User Mismatch

**Pattern:** systemd service running as a different user than the PipeWire session owner. PipeWire is per-user — a `soundsync` system user cannot access another user's PipeWire.

**Rule:** Service must run as `$SUDO_USER` (the human who ran the installer).

### 6.7 Installer Script Regressions

**Pattern:** Multiple fixes for: not stopping service before copy, wrong webui paths, deploying unbuilt source instead of dist/, build order issues.

**Rule:** After installer changes, verify: (1) service stops, (2) binary copies, (3) webui dist/ copies (not src/), (4) service restarts.

---

## 7. Things That Must Always Be True (Invariants)

1. **No custom A2DP endpoints registered** — WirePlumber owns Bluetooth transports
2. **`libspa-0.2-bluetooth` in install.sh** — mandatory for any Bluetooth functionality
3. **Service runs as the PipeWire session user** — not root, not a system user
4. **All 3 version locations in sync** — Cargo.toml, package.json, install.sh
5. **Pre-commit checks pass** — fmt, clippy, test, build — before every push
6. **Null sink set as default** — otherwise Bluetooth audio won't route to capture
7. **Capture uses 3-tier source resolution** — BT direct → null sink monitor → default
8. **webui/dist/ deployed** — never the source index.html

---

## 8. Pre-Commit Checklist

```bash
# Run ALL of these. Do not push if any fail.
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo build
cd webui && npx eslint src/ --ext .ts,.tsx   # if frontend changed
```

---

## 9. Diagnostic Commands (When Audio Breaks)

```bash
# Quick health check on the target machine
echo "=== PipeWire ===" && systemctl --user status pipewire --no-pager 2>&1 | head -3
echo "=== WirePlumber ===" && systemctl --user status wireplumber --no-pager 2>&1 | head -3
echo "=== Sinks ===" && pactl list short sinks
echo "=== Sources ===" && pactl list short sources
echo "=== Default Sink ===" && pactl get-default-sink
echo "=== BT Connected ===" && bluetoothctl devices Connected
echo "=== PW Links ===" && pw-link -l 2>/dev/null | head -40
echo "=== Capture Procs ===" && ps aux | grep -E 'parec|pw-cat' | grep -v grep
echo "=== SoundSync Service ===" && systemctl status soundsync --no-pager 2>&1 | head -10
echo "=== SoundSync Logs ===" && journalctl -u soundsync --no-pager -n 30 2>/dev/null
```

---

## 10. Common Pitfalls (Quick Reference)

| Symptom | Cause | Fix |
|---------|-------|-----|
| No Bluetooth audio at all | `libspa-0.2-bluetooth` missing | `apt install libspa-0.2-bluetooth` |
| `pactl`/`parec` not found | `pulseaudio-utils` missing | `apt install pulseaudio-utils` |
| No BT audio node in PipeWire | WirePlumber A2DP sink config missing | Run installer's `configure_wireplumber()` |
| Null sink not created / silent | Service running as wrong user | Fix systemd `User=` to match PipeWire owner |
| AVRCP metadata missing | Device never reaches AudioActive | Check `pactl list short sources` for `bluez_input.*` |
| "Unknown transport" in WP logs | Custom A2DP endpoints registered | Remove MediaEndpoint1 registration |
| `pw-cat --target` fails | Using PulseAudio names, not PipeWire | Use PipeWire node names |
| Wrong version displayed | Not all version files updated | Update Cargo.toml + package.json + install.sh + version.ts |
| Blank web page after deploy | Deployed source instead of dist/ | Copy `webui/dist/` not `webui/src/` |
| CI fails on push | Didn't run pre-commit checks | Run fmt + clippy + test + build locally first |

---

## 11. Planning Documents

Detailed specs for each subsystem live in `planning/`:
- `1.md` — Bluetooth A2DP Sink
- `2.md` — Audio Pipeline
- `3.md` — Equalizer (10-band, presets)
- `4.md` — WebRTC Streaming
- `5.md` — REST API (37 endpoints)
- `6.md` — WebSocket Events
- `7.md` — AVRCP Media Controls
- `8.md` — Chromecast/AirPlay Output
- `9.md` — Codec Support (SBC, AAC, LDAC, aptX)
- `10.md` — Web UI

Also see: `do-over.md` (54K comprehensive rebuild spec), `COMPARISON.md` (vs StreamCast), `ALWAYSUPDATE.md` (operational checklist).
