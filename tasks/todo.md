# SoundSync Stability Improvements — Implementation Plan

## Problem Statement

SoundSync has several instability issues:
1. Bluetooth connects but phone doesn't see SoundSync as a speaker (A2DP sink role not applied)
2. PipeWire/WirePlumber fail to restart properly
3. System sometimes requires reboot after install/restart
4. Duplicate/orphaned audio nodes accumulate across restarts
5. No self-healing diagnostic tool exists
6. **[NEW]** Audio stuttering during WebRTC streaming
7. **[NEW]** Safari WebRTC audio failure (ICE candidate rejection)

## Root Cause Analysis

### Issue 1: Phone doesn't see SoundSync as speaker
- WirePlumber config may be written but not loaded (restart fails silently)
- BlueZ Class of Device must be 0x240414 (Audio/Video + Rendering + Loudspeaker)
- libspa-0.2-bluetooth may be missing
- Bluetooth adapter may not be in discoverable mode

### Issue 2: PipeWire/WirePlumber restart failures
- User services require proper XDG_RUNTIME_DIR and DBUS_SESSION_BUS_ADDRESS
- systemctl --user from root context fails without su - or machinectl shell
- Service file in scripts/soundsync.service uses hardcoded User=soundsync

### Issue 3: Reboot required
- install.sh restarts WirePlumber but doesn't verify it actually came back
- PipeWire user services may not be enabled for linger
- Runtime dir may not exist after install

### Issue 4: Duplicate nodes
- ExecStartPre cleanup only catches modules with soundsync in args
- Multiple WirePlumber instances can exist
- Orphaned pw-loopback or filter-chain processes from previous runs

### Issue 6: Audio Stuttering (FIXED)
**Root Causes:**
- RTP timestamps not advancing when broadcast channel frames are dropped (Lagged(n))
  - Browser jitter buffer sees timestamps behind wall-clock → plays too fast → stutter
- Opus encoding running on async Tokio threads (CPU-bound blocking)
  - Delays other async tasks, causing cascading latency spikes
- No pacing on RTP packet delivery
  - OS scheduler bursts cause packet clusters instead of smooth 20ms intervals

**Fixes Applied (webrtc_audio.rs):**
1. Advance timestamp by `n * 960` and sequence_number by `n` on Lagged(n) events
2. Move Opus encoding to `spawn_blocking` thread pool
3. Add `tokio::time::interval(20ms)` pacer with `MissedTickBehavior::Skip`
4. Add periodic health logging (frames sent, lag count, uptime)

### Issue 7: Safari WebRTC Failure (FIXED)
**Root Cause:**
- `IceCandidateData` in ws.rs used `#[serde(rename_all = "camelCase")]`
- Serde converts `sdp_mline_index` → `sdpMlineIndex` (lowercase 'l')
- WebRTC spec requires `sdpMLineIndex` (capital 'L')
- Client looked for `sdpMLineIndex` or `sdp_mline_index` — neither matched
- Safari strictly requires at least one of sdpMid/sdpMLineIndex to be non-null
- All server ICE candidates were effectively missing sdpMLineIndex

**Fix Applied (ws.rs):**
- Replaced `rename_all = "camelCase"` with explicit `#[serde(rename = "sdpMid")]` and `#[serde(rename = "sdpMLineIndex")]` on each field

## Implementation

### Phase 1: Build soundsync-doctor.sh (comprehensive diagnostic + repair) ✅
### Phase 2: Harden install.sh (verify restarts, auto-recover) ✅
### Phase 3: Fix service file template ✅
### Phase 4: Fix audio streaming issues ✅
### Phase 5: Validate, commit, push
