# Lessons Learned

## D-Bus Connections Go Stale
**Pattern:** Long-running D-Bus connections (zbus) silently fail after Bluetooth adapter resets, system suspend/resume, or D-Bus service restarts. All proxy calls fail silently.
**Rule:** Always track consecutive D-Bus failures and reconnect after a threshold. Never assume a D-Bus connection established at startup will remain valid indefinitely.

## systemd After= Is Not Readiness
**Pattern:** `After=pipewire.service` only guarantees start ordering, NOT that PipeWire is ready to accept commands (e.g., `pactl info`).
**Rule:** For services that depend on audio infrastructure, add an explicit readiness probe (loop with `pactl info`) in both the app initialization and systemd ExecStartPre.

## Silent Error Swallowing
**Pattern:** `let _ = channel.send(...)` and `if let Err(e) = ... { return }` patterns hide systemic failures.
**Rule:** At minimum, log at warn level when channel sends or D-Bus calls fail. Track failure patterns to detect degradation early.

## Virtual Audio Devices Don't Persist Across Reboots
**Pattern:** PipeWire null sinks, loopback modules, and filter-chains created at runtime are ephemeral.
**Rule:** Always recreate the full audio graph on startup. Never assume previous-session state exists.
