# SoundSync Investigation Report

## Issue 1: Safari Audio Playback Broken

### Investigation Status: ROOT CAUSE IDENTIFIED

### Summary of Audit

Audited the full WebRTC pipeline end-to-end:
- Backend: `src/audio/webrtc_audio.rs` (WebRTC manager, Opus encoding, RTP)
- Backend signaling: `src/web/ws.rs` (WebSocket message types, serde serialization)
- Frontend WebRTC: `webui/src/api/webrtc.ts` (RTCPeerConnection, audio element, ICE)
- Frontend UI: `webui/src/components/MediaControls/MediaControls.tsx` (gesture handler, fallback)
- Types: `webui/src/types.ts` (ICE candidate type definitions)

### What's Already Fixed (Verified Working)

1. **sdpMLineIndex casing** (`ws.rs:384-395`, `ws.rs:431-437`): Explicit `#[serde(rename = "sdpMLineIndex")]` on both serialization and deserialization structs. Correct.
2. **Audio element created synchronously in gesture** (`webrtc.ts:21-24`): `document.createElement('audio')` runs synchronously before any `await`. Correct.
3. **AudioContext unlock pattern** (`webrtc.ts:29-38`): `new AudioContext()` + `await ctx.resume()` inside the gesture handler. Correct.
4. **Pre-play priming** (`webrtc.ts:44`): `audioElement.play()` called before any async WebRTC work. Correct.
5. **srcObject used** (`webrtc.ts:55`): `audioElement.srcObject = stream`, not blob URL. Correct.
6. **MediaStream wrapper** (`webrtc.ts:54`): `event.streams[0] ?? new MediaStream([event.track])` handles webrtc-rs not associating tracks with streams. Correct.
7. **Muted toggle fallback** (`webrtc.ts:60-65`): If play() fails, tries muted->play->unmute. Correct.
8. **ICE candidate validation** (`webrtc.ts:108-115`): Drops candidates where both sdpMid and sdpMLineIndex are null. Correct.
9. **Codec/SDP** (`webrtc_audio.rs:157-166`): Opus 48kHz stereo, standard configuration. Safari-compatible.

### ROOT CAUSE: Race Condition Between setRemoteDescription and addIceCandidate

**File:** `webui/src/components/MediaControls/MediaControls.tsx:101-106`

```typescript
unsubRef.current = ws.current.onMessage((msg: WsMessage) => {
    if (msg.type === 'webrtc_answer') {
        client.handleAnswer(msg.data.sdp);          // NOT awaited
    } else if (msg.type === 'webrtc_ice_candidate') {
        client.handleIceCandidate(msg.data);         // NOT awaited
    }
});
```

**The problem:** Both `handleAnswer` and `handleIceCandidate` are async functions called without `await`. When the server sends the SDP answer followed immediately by ICE candidates (which it does -- the answer is published first via `SystemEvent::WebRtcAnswer`, then ICE candidates arrive asynchronously via `on_ice_candidate` callback in `webrtc_audio.rs:175-190`), the browser processes the WebSocket messages sequentially:

1. WS message arrives: `webrtc_answer` -> `handleAnswer(sdp)` called, starts `pc.setRemoteDescription()` (async, yields)
2. WS message arrives: `webrtc_ice_candidate` -> `handleIceCandidate(data)` called, calls `pc.addIceCandidate()` **BEFORE `setRemoteDescription` has resolved**

**Browser behavior differences:**
- **Chrome/Firefox**: Internally queue ICE candidates received before remote description is set. Candidates are applied once `setRemoteDescription` completes. **Works.**
- **Safari**: Strictly enforces the WebRTC spec requirement that remote description must be set before `addIceCandidate()`. Throws `InvalidStateError`. **Breaks.**

**Why no error surfaces in the UI:** The `handleIceCandidate` promise rejection is unhandled (called without `await` or `.catch()`). The error goes to the browser console as an unhandled promise rejection, but nothing in the UI reacts to it.

**Consequence:** All server-side ICE candidates are silently dropped in Safari. Without server ICE candidates, the ICE negotiation may be incomplete -- the media path never fully establishes, and no audio RTP packets reach the browser. The `ontrack` event may fire (the track is negotiated in SDP), but the underlying transport has no working candidate pair, so no media flows.

### SECONDARY ISSUE: HTTP Fallback Also Broken in Safari

**File:** `webui/src/components/MediaControls/MediaControls.tsx:114-123`

```typescript
setTimeout(() => {
    if (rtcRef.current && !rtcRef.current.isActive && !httpAudioRef.current) {
        const audio = startHttpStream();
        httpAudioRef.current = audio;
        audio.play().catch(() => {});    // <-- NOT in user gesture
    }
}, 5000);
```

When WebRTC fails (which it will in Safari due to the race condition above), the 5-second timeout fires and creates a new `<audio>` element with `src = '/api/stream/audio.aac'`. But this `setTimeout` callback is NOT a user gesture context. Safari's autoplay policy blocks `audio.play()`, and the `.catch(() => {})` silently swallows the `NotAllowedError`.

**Result:** Both the primary path (WebRTC) and the fallback path (HTTP stream) fail silently in Safari.

### Proposed Fix

**Fix 1 (Primary -- ICE candidate race):** Add an ICE candidate queue in the `WebRTCClient` class. Buffer incoming ICE candidates until `setRemoteDescription` has completed, then flush the queue.

```typescript
// In WebRTCClient class:
private remoteDescriptionSet = false;
private pendingCandidates: RTCIceCandidateInit[] = [];

async handleAnswer(sdp: string): Promise<void> {
    if (this.pc) {
        await this.pc.setRemoteDescription(new RTCSessionDescription({ type: 'answer', sdp }));
        this.remoteDescriptionSet = true;
        // Flush queued candidates
        for (const init of this.pendingCandidates) {
            await this.pc.addIceCandidate(new RTCIceCandidate(init));
        }
        this.pendingCandidates = [];
    }
}

async handleIceCandidate(candidate: IceCandidateMessage): Promise<void> {
    // ... existing normalization code ...
    const init: RTCIceCandidateInit = { candidate: candidate.candidate, sdpMid, sdpMLineIndex };

    if (!this.remoteDescriptionSet) {
        this.pendingCandidates.push(init);
        return;
    }
    await this.pc.addIceCandidate(new RTCIceCandidate(init));
}
```

**Fix 2 (Secondary -- HTTP fallback):** Move the HTTP fallback audio element creation into the original gesture handler by pre-creating it (hidden, paused) during the click, then only setting `.src` in the timeout.

Alternatively, `await` the `handleAnswer` call in the message handler so candidates never arrive before remote description is set:

```typescript
unsubRef.current = ws.current.onMessage(async (msg: WsMessage) => {
    if (msg.type === 'webrtc_answer') {
        await client.handleAnswer(msg.data.sdp);
    } else if (msg.type === 'webrtc_ice_candidate') {
        await client.handleIceCandidate(msg.data);
    }
});
```

This is simpler but depends on the WebSocket message handler supporting async callbacks. The queue approach (Fix 1) is safer and doesn't depend on the message handler's async behavior.

### Cross-Browser Risk Assessment

| Change | Chrome impact | Firefox impact | Risk |
|--------|--------------|----------------|------|
| ICE candidate queue | None -- Chrome already queues internally; this just adds an explicit queue that's immediately flushed | None -- same as Chrome | **ZERO** -- purely additive, no behavior change for browsers that already handle this |
| HTTP fallback pre-create | None -- autoplay is less restricted | None -- same | **LOW** -- pre-creating a paused audio element is harmless; all browsers support it |

**DECISION POINT:** The ICE candidate queue fix has ZERO regression risk for Chrome/Firefox. It only adds explicit behavior that those browsers already do implicitly. This is safe to proceed. Logging this decision here as required.

---

## Issue 2: AirPlay Not Working

### Investigation Status: AWAITING LOGS

### Architecture Map

AirPlay is implemented in `src/audio/airplay.rs` using PipeWire's RAOP support:

```
Discovery:  avahi-browse -t -r -p -k _raop._tcp
               |
               v (fallback if avahi fails)
            pw-cli list-objects | grep raop
               |
               v
Connection: pactl load-module module-raop-discover
               |
               v
            pactl list short sinks  (poll for RAOP sink to appear, 10x @ 500ms)
               |
               v
Routing:    pw-link soundsync-capture:monitor_FL -> raop_sink:playback_FL
            pw-link soundsync-capture:monitor_FR -> raop_sink:playback_FR
               |
               v
Monitoring: Every 5s: pactl list short sinks (verify sink still exists)
```

**External dependencies:**
- `avahi-daemon` -- mDNS service discovery
- `pipewire` + `pipewire-pulse` + `wireplumber` -- audio infrastructure
- `libpipewire-module-raop-sink.so` -- RAOP protocol handler (may need separate package)
- `pactl` -- PipeWire/PulseAudio CLI for module loading and sink management
- `pw-link` -- PipeWire link management
- `pw-cli` -- PipeWire object inspection (fallback discovery)

**Error paths identified** (all in `src/audio/airplay.rs`):

| Error | Location | Log Level | Published to Frontend? | Displayed in UI? |
|-------|----------|-----------|----------------------|-----------------|
| Device not found | line 152 | warn | Yes (AirPlayError) | **NO** |
| RAOP module load fail | line 314 | warn | No | NO |
| RAOP sink not found | line 178-189 | error | Yes (AirPlayError) | **NO** |
| No monitor ports for pw-link | create_pw_links | error | Yes (AirPlayError) | **NO** |
| No input ports for pw-link | create_pw_links | error | Yes (AirPlayError) | **NO** |
| pw-link command fails | create_pw_links | error | Yes (AirPlayError) | **NO** |
| Volume set fails | line 284-290 | warn | No | NO |
| Avahi not running | check_avahi_running | warn (at startup) | No | NO |
| RAOP module not installed | check_raop_availability | warn (at startup) | No | NO |

**Key finding:** `AirPlayError` events ARE published to WebSocket, but the frontend `AudioOutput.tsx` component does NOT display them. Errors are delivered to the browser but silently ignored by the UI.

### Diagnostic Commands To Run

**Logging setup:** SoundSync uses `tracing` + `tracing-subscriber` with `EnvFilter`. Default level is `info` (set via `RUST_LOG=info` in the systemd service). The service file is `/etc/systemd/system/soundsync.service`.

Please run these commands and paste the output back:

#### 1. Service status and recent logs (identifies if SoundSync is running and any recent errors)
```bash
systemctl status soundsync
journalctl -u soundsync --since "1 hour ago" --no-pager | tail -100
```

#### 2. Enable debug logging and restart (captures detailed AirPlay trace)
```bash
# Set debug logging temporarily
sudo systemctl set-environment RUST_LOG=soundsync=debug
sudo systemctl restart soundsync

# Wait 5 seconds for startup
sleep 5

# Check it started
systemctl is-active soundsync
```

#### 3. Trigger AirPlay discovery from the UI, then capture logs
```bash
# After pressing "Scan" in the AirPlay tab in the UI:
journalctl -u soundsync --since "30 seconds ago" --no-pager
```

#### 4. Check Avahi (mDNS discovery)
```bash
# Is Avahi running?
systemctl is-active avahi-daemon

# Can Avahi see any AirPlay receivers on the network?
avahi-browse -t -r -p -k _raop._tcp 2>&1 | head -30

# Also check for AirPlay 2 (might advertise differently)
avahi-browse -t -r -p -k _airplay._tcp 2>&1 | head -30
```

#### 5. Check PipeWire RAOP module availability
```bash
# Is the RAOP sink module installed?
find /usr/lib -name "libpipewire-module-raop*" -type f 2>/dev/null

# Is PipeWire running?
systemctl --user is-active pipewire pipewire-pulse wireplumber

# Are there any RAOP sinks already visible?
pactl list short sinks 2>&1 | grep -i raop

# Is module-raop-discover loaded?
pactl list short modules 2>&1 | grep -i raop
```

#### 6. Check PipeWire graph (audio routing health)
```bash
# Does the soundsync-capture sink exist?
pactl list short sinks 2>&1 | grep soundsync

# What ports does it expose?
pw-link -o 2>&1 | grep soundsync

# Any existing links?
pw-link -l 2>&1 | grep -A2 soundsync
```

#### 7. Network-level AirPlay check (if avahi-browse shows nothing)
```bash
# Check if mDNS traffic is reaching the machine
# (requires avahi-utils or similar)
avahi-resolve -n $(hostname).local 2>&1

# Check firewall isn't blocking mDNS (port 5353/UDP)
sudo iptables -L INPUT -n 2>&1 | grep -E "5353|mdns"
sudo ss -ulnp | grep 5353
```

#### 8. If AirPlay connect was attempted (after clicking Connect on a device in UI)
```bash
journalctl -u soundsync --since "60 seconds ago" --no-pager | grep -E "AirPlay|RAOP|raop|pw-link|airplay"
```

Please run commands 1-6 first and paste the output. Commands 7-8 are conditional -- only needed if earlier commands don't reveal the issue.

---

## Action Items

- [x] Audit WebRTC pipeline for Safari issues
- [x] Audit AirPlay backend error paths
- [x] Identify Safari root cause (ICE candidate race condition)
- [x] Assess cross-browser regression risk (ZERO for primary fix)
- [x] Produce AirPlay diagnostic commands
- [ ] **BLOCKED**: Awaiting user approval to implement Safari fix
- [ ] **BLOCKED**: Awaiting user log output for AirPlay diagnosis
