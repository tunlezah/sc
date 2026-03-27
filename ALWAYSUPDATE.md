# ALWAYSUPDATE.md — Context for AI Assistants

This file provides critical context for AI coding assistants working on SoundSync.
Feed this file as context at the start of every session.

## Version Locations

When bumping version numbers, ALL of these must be updated together:

| File | Location | Format | Notes |
|------|----------|--------|-------|
| `Cargo.toml` | Line ~3: `version = "X.Y.Z"` | `"X.Y.Z"` | Backend version. `env!("CARGO_PKG_VERSION")` reads this at compile time — do NOT use environment variables. |
| `webui/package.json` | Line ~4: `"version": "X.Y.Z"` | `"X.Y.Z"` | Frontend npm version. |
| `webui/package-lock.json` | Lines ~3 and ~9: `"version": "X.Y.Z"` | `"X.Y.Z"` | Lock file — TWO occurrences at top of file. |
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

## Architecture Notes

### Audio Pipeline

The audio pipeline depends on PipeWire and its ecosystem:

- **Null Sink**: Created via `pactl load-module module-null-sink` (requires `pulseaudio-utils` package) or `pw-loopback` as fallback
- **Default Sink**: Must be set to `soundsync-capture` so Bluetooth audio routes there
- **Capture**: Uses `parec` (preferred, from `pulseaudio-utils`) or `pw-cat` (PipeWire native)
- **A2DP Sink Role**: Requires WirePlumber to be configured with `bluez5.roles = [ a2dp_sink ]`

### Bluetooth A2DP

The system acts as a Bluetooth A2DP **sink** (receives audio, like a speaker). This requires:

1. **BlueZ** agent registered for auto-pairing
2. **WirePlumber** configured with A2DP sink role — it handles codec negotiation, transport acquisition, and creates `bluez_input.*` PipeWire audio nodes
3. **PipeWire** routing from the Bluetooth source to the capture null sink

Custom `MediaEndpoint1` registration in `src/bluetooth/endpoint.rs` **conflicts** with WirePlumber's
built-in BlueZ plugin. If both register endpoints, BlueZ uses one set and the other can't acquire
the transport. Current approach: let WirePlumber handle A2DP entirely.

### Required System Packages

```
bluetooth bluez
pipewire pipewire-pulse wireplumber pulseaudio-utils
libdbus-1-dev libpipewire-0.3-dev libspa-0.2-dev
libclang-dev libopus-dev libmp3lame-dev pkg-config build-essential
avahi-daemon avahi-utils
ffmpeg
git curl unzip
```

### Service Architecture

SoundSync runs as a **systemd system service** under the `soundsync` user.
PipeWire is a **per-user session service**. The `soundsync` user needs:
- `loginctl enable-linger` enabled
- Its own PipeWire and WirePlumber sessions running
- `XDG_RUNTIME_DIR` set correctly

### WirePlumber A2DP Configuration

WirePlumber must be configured to act as an A2DP sink. Without this, Bluetooth devices
won't see the machine as a speaker. Config file location depends on WirePlumber version:

- **WirePlumber 0.4.x** (Lua): `/etc/wireplumber/bluetooth.lua.d/51-soundsync.lua`
- **WirePlumber 0.5+** (conf): `/etc/wireplumber/wireplumber.conf.d/51-soundsync.conf`

## Diagnostic Commands

When audio isn't working, run this on the target machine:

```bash
# Full diagnostic
echo "=== PipeWire ===" && systemctl --user status pipewire --no-pager 2>&1 | head -3
echo -e "\n=== WirePlumber ===" && systemctl --user status wireplumber --no-pager 2>&1 | head -3
echo -e "\n=== Sinks ===" && pactl list short sinks
echo -e "\n=== Sources ===" && pactl list short sources
echo -e "\n=== Default Sink ===" && pactl get-default-sink
echo -e "\n=== BT Connected ===" && bluetoothctl devices Connected
echo -e "\n=== PW Links ===" && pw-link -l 2>/dev/null | head -40
echo -e "\n=== Capture Procs ===" && ps aux | grep -E 'parec|pw-cat' | grep -v grep
echo -e "\n=== WP BT Config ===" && cat /etc/wireplumber/bluetooth.lua.d/51-soundsync.lua 2>/dev/null || cat /etc/wireplumber/wireplumber.conf.d/51-soundsync.conf 2>/dev/null || echo "No WP A2DP config found"
echo -e "\n=== SoundSync Logs ===" && journalctl -u soundsync --no-pager -n 30 2>/dev/null
```

## Common Pitfalls

1. **Version not showing in UI**: `webui/src/version.ts` is the source, not `package.json`
2. **pactl not found**: Install `pulseaudio-utils` (separate from `pipewire-pulse`)
3. **No BT audio node**: WirePlumber needs A2DP sink config (see above)
4. **Null sink not created**: Check PipeWire is running for the service user
5. **pw-cat --target fails**: Use PipeWire node names, not PulseAudio names
6. **OS version in build**: Never use `$(uname -r)` or env vars for app version
