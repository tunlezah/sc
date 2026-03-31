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

## systemctl --user From Root Requires Full Environment
**Pattern:** Running `systemctl --user restart wireplumber` from a root shell (e.g., installer running as sudo) fails silently because XDG_RUNTIME_DIR and DBUS_SESSION_BUS_ADDRESS are not set for the target user.
**Rule:** Always use `su - $USER -s /bin/bash -c "export XDG_RUNTIME_DIR=...; export DBUS_SESSION_BUS_ADDRESS=...; systemctl --user ..."` when managing user services from root context. Never assume `systemctl --user` will work without these environment variables.

## "Config Written" Does Not Mean "Config Loaded"
**Pattern:** Writing a WirePlumber config file to /etc/wireplumber/ and assuming it takes effect. WirePlumber must be restarted AND verified active for config to be loaded. A failed restart means the old config (or no config) is still active.
**Rule:** After writing any config, restart the consuming service AND verify it is running. If verification fails, attempt recovery before declaring success.

## Service Restart Verification Is Mandatory
**Pattern:** `systemctl restart foo || warn "failed"` — the warning is logged but installation continues as if everything is fine. Downstream steps assume the service is running.
**Rule:** After every service restart in an installer or repair script, poll `is-active` with a timeout. If the service doesn't come back, attempt recovery (fix environment, daemon-reload, retry). Never proceed with dependent steps if a prerequisite service is down.

## Duplicate Nodes Accumulate Across Restarts
**Pattern:** PipeWire modules (null-sink, loopback) with "soundsync" in their args accumulate when the service crashes and restarts without cleanup. Each restart creates new modules without removing old ones.
**Rule:** Always clean up SoundSync-owned modules before creating new ones. The ExecStartPre in the systemd service handles this, but any repair/diagnostic tool must also detect and remove duplicates.

## Service File Template vs Generated File Drift
**Pattern:** The service file template in scripts/soundsync.service drifts from what install.sh actually generates. Missing environment variables (DBUS_SESSION_BUS_ADDRESS, PULSE_RUNTIME_PATH) or wrong User= cause silent failures.
**Rule:** Keep the template as a reference with REPLACE_* placeholders. The install.sh generated version is the source of truth. Document this clearly in the template.
