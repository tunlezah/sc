# SoundSync UI Redesign & Bug Fix Task Tracker

## Completed

- [x] UI Redesign: 65/35 two-column dashboard layout
- [x] Logo integration (header + favicon)
- [x] DeviceList search/filter bar
- [x] AudioOutput Line-In tab
- [x] MediaControls progress bar with timestamps
- [x] Bug Fix 1: Stale metadata — AVRCP D-Bus reconnection
- [x] Bug Fix 2: Stale play/pause — AVRCP D-Bus connection recovery
- [x] Bug Fix 3: Post-reboot audio — PipeWire readiness probing + systemd deps
- [x] Updated systemd service with proper dependency ordering

## Review

### Changes Made
1. **UI Layout** — Grid changed from 50/50 to 65/35 split, gap/padding tweaked
2. **Logo** — SoundSyncLogo.png replaces StreamCastImage.png in header and favicon
3. **DeviceList** — Added search/filter input to filter by name or MAC address
4. **AudioOutput** — Added Line-In tab alongside Chromecast/AirPlay
5. **MediaControls** — Added progress bar, elapsed/total timestamps, track source label
6. **AVRCP Monitor** — Added consecutive failure tracking + automatic D-Bus reconnect
7. **Audio Pipeline** — Added `wait_for_audio_services()` pre-check with 30s timeout
8. **Null Sink Creation** — Increased retries from 5 to 10 with exponential backoff
9. **systemd Service** — Added `After=pipewire-pulse.service wireplumber.service`, PipeWire readiness pre-check

### Root Cause Analysis

**Bug 1 (Stale Metadata) + Bug 2 (Stale Play/Pause):**
Same root cause — the AVRCP monitor creates a single D-Bus connection at startup and never reconnects. After prolonged runtime (adapter resets, D-Bus service restarts), the connection goes stale. All polls and commands silently fail. Fix: track consecutive failures and reconnect after 5 consecutive D-Bus errors.

**Bug 3 (Audio Broken After Reboot):**
The systemd service uses `After=pipewire.service` but PipeWire/WirePlumber may not be fully ready to accept `pactl` commands when the unit starts. The null sink creation would retry 5 times with 1s delays — insufficient on slow systems. Fix: (1) explicit readiness probe loop before pipeline init, (2) doubled retries with exponential backoff, (3) systemd ExecStartPre readiness check, (4) proper After= for all PipeWire units.

### Remaining Risks
- No integration test coverage for D-Bus reconnection (requires live BlueZ)
- Systemd `After=` is ordering only, not readiness — the app-level probe mitigates this
- If PipeWire never becomes ready within 30s, pipeline still attempts init (logs warning)
