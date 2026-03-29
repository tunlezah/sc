# Bluetooth Improvements

Analysis of the SoundSync Bluetooth implementation (`src/bluetooth/`).

## Current Architecture

```
Discovered → Pairing → Paired → Connected → ProfileNegotiated → PipewireSourceReady → AudioActive
```

- **Manager** (`manager.rs`): Connection lifecycle, command processing, device polling
- **Discovery** (`discovery.rs`): State tracking, event publishing
- **Device** (`device.rs`): 7-state machine for device lifecycle
- **Agent** (`agent.rs`): Auto-pairing via D-Bus Agent1
- **AVRCP** (`avrcp.rs`): Media controls + metadata extraction
- **Codecs** (`codecs/`): SBC, AAC, LDAC, aptX, aptX HD capability definitions
- **Endpoint** (`endpoint.rs`): A2DP MediaEndpoint1 (dead code — WirePlumber handles this)

## High Priority Improvements

### 1. Automatic Reconnection (Missing)

Currently, when a device disconnects, state transitions to `Disconnected` and the user must manually reconnect. There is no retry mechanism.

**Recommendation:** On disconnect of a trusted device, attempt reconnection with exponential backoff (1s, 2s, 4s, 8s, max 3 attempts). Emit `Reconnecting` state to UI so users see progress.

```rust
// Pseudocode for reconnection
async fn attempt_reconnect(&self, address: &str) {
    for attempt in 1..=3 {
        let delay = Duration::from_secs(1 << (attempt - 1));
        tokio::time::sleep(delay).await;
        if device.connect().await.is_ok() {
            return; // Success
        }
    }
    // Give up, publish error
}
```

### 2. Operation Timeouts (Missing)

`device.connect()`, `adapter.remove_device()`, and other D-Bus calls have no timeout wrapper. If BlueZ hangs, the manager blocks indefinitely.

**Recommendation:** Wrap all D-Bus operations in `tokio::time::timeout`:
- Connect: 10s
- Disconnect: 5s
- Remove: 5s
- Set property: 3s

### 3. active_device Tracking

`active_device` is only set when a device reaches `AudioActive` state. AVRCP depends on `active_device` being set, but if the PipeWire source detection is slow, AVRCP controls won't work for several seconds after connection.

**Recommendation:** Set `active_device` when device reaches `Connected` state, not just `AudioActive`.

## Medium Priority Improvements

### 4. Forced State Transitions

`discovery.rs` forces invalid state transitions with a warning:
```rust
warn!("Forced device {} state: {:?} → {:?}", ...);
device.state = new_state.clone(); // Force anyway!
```

This masks bugs. Invalid transitions should be logged as errors and investigated.

### 5. Device Removal Cleanup

When removing a device:
- BlueZ pairing is removed ✓
- App state is cleaned ✓
- Endpoint D-Bus objects are NOT unregistered ✗
- WirePlumber connections rely on BlueZ disconnect callback ✓ (works but implicit)

### 6. Error Message Quality

Current error messages forward raw BlueZ errors without context. Should distinguish:
- "Device not found" → "Device may be out of range"
- "Connection timeout" → "Device did not respond. Ensure it's in pairing mode."
- "Adapter unavailable" → "Bluetooth hardware not detected. Check that the adapter is connected."

## Low Priority Improvements

### 7. Agent Security

The pairing agent always approves authorization requests. On untrusted networks, this could allow unauthorized service access. Consider adding a UUID whitelist for A2DP/AVRCP profiles only.

### 8. Codec Preference

No user-configurable codec preference. When multiple codecs are supported, the system relies on WirePlumber's default selection. Consider allowing users to prefer LDAC over SBC for higher quality.

### 9. Connection Quality Monitoring

RSSI is captured during discovery but not monitored during active connections. Tracking RSSI during polling would allow proactive reconnection before signal loss.

## Current Timing Constants

| Constant | Value | Purpose |
|----------|-------|---------|
| `DEVICE_PROPS_POLL` | 500ms | Device status polling |
| `AVRCP_POLL_ACTIVE` | 250ms | Media player polling (connected) |
| `AVRCP_POLL_IDLE` | 2000ms | Media player polling (idle) |

## Dead Code Note

`endpoint.rs` implements A2DP `MediaEndpoint1` D-Bus interface but is intentionally NOT registered. WirePlumber's BlueZ plugin handles codec negotiation and transport acquisition. Registering custom endpoints would conflict with WirePlumber, causing "unknown transport" errors. This is documented in `main.rs` lines 139-153.

## Summary

| Aspect | Status | Risk |
|--------|--------|------|
| Connection lifecycle | Working | Low |
| Auto-reconnection | Missing | High — users must manually reconnect |
| Operation timeouts | Missing | High — manager can hang |
| Device removal | Mostly clean | Low |
| Error handling | Inconsistent | Medium |
| AVRCP | Working but delayed | Medium |
| Codec support | Complete | Low |
