# ALWAYSUPDATE.md — Context for AI Assistants

This file provides critical context for AI coding assistants working on SoundSync.
Feed this file as context at the start of every session.

## Version Locations

When bumping version numbers, ALL of these must be updated together:

| File | Location | Format | Notes |
|------|----------|--------|-------|
| `Cargo.toml` | Line ~3: `version = "X.Y.Z"` | `"X.Y.Z"` | Backend version. `env!("CARGO_PKG_VERSION")` reads this at compile time — do NOT use environment variables. |
| `webui/package.json` | Line ~4: `"version": "X.Y.Z"` | `"X.Y.Z"` | Frontend npm version. |
| `webui/src/version.ts` | Line 1: `export const VERSION = 'vX.Y.Z';` | `'vX.Y.Z'` | **Displayed in the web UI header.** Note the `v` prefix. |
| `install.sh` | Line ~15: `VERSION="X.Y.Z"` | `"X.Y.Z"` | Shown during install, written to version file. |

**WARNING**: Do NOT use `$(cargo pkgid)`, shell commands, or environment variables to
derive the version at runtime in install.sh — this can accidentally pick up the OS
version or Cargo registry metadata. Always hardcode the string.

## Pre-Commit Checklist

Before committing AND before pushing, ALWAYS run these locally and ensure they pass:

```bash
# 1. Format check (must produce no output)
cargo fmt --all --check

# 2. Clippy with strict warnings (must produce no errors)
cargo clippy --all-targets --all-features -- -D warnings

# 3. All tests pass
cargo test

# 4. Build succeeds
cargo build
```

Do NOT push code that fails any of these checks. Fix issues locally first.

## README.md Maintenance

The `README.md` must always reflect the current state of the project. After any
significant change, review and update:

- Feature list
- Architecture diagram
- API endpoint table
- System package requirements
- Project structure tree
- Changelog (add new version entry)
- Troubleshooting section

## Install Script (`install.sh`) Maintenance

The install script must include ALL system dependencies the project needs.
When adding new functionality that requires external tools or libraries,
**immediately** update `install.sh` to install them. Key areas:

### Required System Packages (must ALL be in install.sh)

```
# Bluetooth stack
bluetooth bluez libspa-0.2-bluetooth

# Audio pipeline
pipewire pipewire-pulse wireplumber pulseaudio-utils

# Build dependencies
libdbus-1-dev libpipewire-0.3-dev libspa-0.2-dev
libclang-dev libopus-dev libmp3lame-dev pkg-config build-essential

# Network discovery
avahi-daemon avahi-utils

# Audio encoding
ffmpeg

# General tools
git curl unzip
```

Critical packages often missed:
- `libspa-0.2-bluetooth` — PipeWire BlueZ SPA plugin. Without this, WirePlumber cannot interact with Bluetooth AT ALL. No A2DP, no audio nodes, nothing.
- `pulseaudio-utils` — Provides `pactl` and `parec` CLI tools. Separate from `pipewire-pulse`.
- `ffmpeg` — Required for AAC-LC encoding.

### WirePlumber Configuration

The install script must configure WirePlumber for A2DP sink role. Without this config,
Bluetooth devices cannot stream audio to the machine. See `configure_wireplumber()` in install.sh.

- WirePlumber 0.4.x uses Lua files in `/etc/wireplumber/bluetooth.lua.d/`
- WirePlumber 0.5+ uses conf files in `/etc/wireplumber/wireplumber.conf.d/`

### Service User

The systemd service MUST run as the same user that has PipeWire running.
PipeWire is a per-user service — a separate `soundsync` system user cannot
see PipeWire nodes, create sinks, or capture audio from another user's session.
The install script uses `$SUDO_USER` (the user who ran `sudo bash install.sh`).

## Architecture Notes

### Audio Pipeline

The audio pipeline depends on PipeWire and its ecosystem:

- **Null Sink**: Created via `pactl load-module module-null-sink` or `pw-loopback` fallback
- **EQ Filter-Chain**: `pipewire-filter-chain` subprocess creates `effect_input.soundsync-eq` (Audio/Sink) that outputs to `soundsync-capture`
- **Default Sink**: When EQ is enabled, default sink must be `effect_input.soundsync-eq` so audio flows through EQ. When disabled, default is `soundsync-capture`.
- **Capture**: Uses `parec` (preferred) or `pw-cat` (fallback), always from `soundsync-capture.monitor` to receive EQ-processed audio. Direct BT capture is only a fallback if null sink doesn't exist.
- **EQ Update Pipeline**: Web API sends `PipelineCommand::UpdateEq` via mpsc channel to the pipeline task, which restarts `pipewire-filter-chain` with new settings.

### Bluetooth A2DP

The system acts as a Bluetooth A2DP **sink** (receives audio, like a speaker). This requires:

1. **BlueZ** agent registered for auto-pairing (`src/bluetooth/agent.rs`)
2. **WirePlumber** with `libspa-0.2-bluetooth` — handles codec negotiation, transport acquisition, and creates `bluez_input.*` PipeWire audio nodes
3. **PipeWire** routing from the Bluetooth source to the capture null sink

**Custom A2DP endpoints MUST NOT be registered.** The code in `src/bluetooth/endpoint.rs`
is kept for reference but is not used. Registering custom MediaEndpoint1 objects with BlueZ
causes BlueZ to assign A2DP transports to our endpoints instead of WirePlumber's. Since our
endpoints never call `Acquire()`, WirePlumber sees the transport as "unknown" and cannot
create `bluez_input.*` audio nodes. This results in WirePlumber logging
"Properties changed in unknown transport" and no audio flowing.

Instead, device state transitions to `AudioActive` are detected by polling for
`bluez_input.*` / `bluez_source.*` PipeWire sources via `pactl` in the Bluetooth
manager's `poll_device_properties()` method.

### AVRCP

The AVRCP monitor (`src/bluetooth/avrcp.rs`) polls BlueZ for playback status and
track metadata. It depends on the device state being set to `AudioActive`, which
the Bluetooth manager now detects by polling for `bluez_input.*` PipeWire sources
(previously this was set by the now-disabled A2DP endpoint `SetConfiguration` callback).

## Diagnostic Commands

When audio isn't working, run this on the target machine:

```bash
echo "=== PipeWire ===" && systemctl --user status pipewire --no-pager 2>&1 | head -3
echo -e "\n=== WirePlumber ===" && systemctl --user status wireplumber --no-pager 2>&1 | head -3
echo -e "\n=== Sinks ===" && pactl list short sinks
echo -e "\n=== Sources ===" && pactl list short sources
echo -e "\n=== Default Sink ===" && pactl get-default-sink
echo -e "\n=== BT Connected ===" && bluetoothctl devices Connected
echo -e "\n=== PW Links ===" && pw-link -l 2>/dev/null | head -40
echo -e "\n=== Capture Procs ===" && ps aux | grep -E 'parec|pw-cat' | grep -v grep
echo -e "\n=== SoundSync Service ===" && systemctl status soundsync --no-pager 2>&1 | head -10
echo -e "\n=== SoundSync Logs ===" && journalctl -u soundsync --no-pager -n 30 2>/dev/null
echo -e "\n=== WP BT Config ===" && cat /etc/wireplumber/bluetooth.lua.d/51-soundsync.lua 2>/dev/null || echo "No WP config"
echo -e "\n=== BT SPA Plugin ===" && find /usr/lib -name "spa-0.2" -type d 2>/dev/null | head -1 | xargs -I{} ls {}/bluez5/ 2>/dev/null || echo "No BlueZ SPA"
```

## Common Pitfalls

1. **No Bluetooth audio at all**: `libspa-0.2-bluetooth` not installed
2. **pactl/parec not found**: `pulseaudio-utils` not installed
3. **No BT audio node**: WirePlumber A2DP sink config missing
4. **Null sink not created / capture silent**: Service running as wrong user (different PipeWire session)
5. **AVRCP metadata missing**: Device never reaches `AudioActive` state (check `pactl list short sources` for `bluez_input.*`)
6. **pw-cat --target fails**: Use PipeWire node names, not PulseAudio names
7. **OS version in build**: Never use `$(uname -r)` or env vars for app version
8. **Version not updated**: Check ALL locations listed above
9. **"Unknown transport" in WirePlumber logs**: Custom A2DP MediaEndpoint1 objects are registered with BlueZ, stealing transports from WirePlumber. Do NOT register custom A2DP endpoints — WirePlumber must own the transports to create audio nodes. See `src/main.rs` comment.
10. **EQ has no effect on audio**: Default sink must be `effect_input.soundsync-eq` when EQ is enabled. If it's `soundsync-capture`, audio bypasses EQ entirely.
11. **EQ changes don't apply until restart**: EQ updates from the web API must reach the pipeline via `PipelineCommand::UpdateEq`. If `pipeline_cmd_tx` is not wired into the `AppRouter`, changes only update the UI state.
