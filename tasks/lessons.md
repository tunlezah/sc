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

## No BlueZ5 Device in PipeWire = No Audio
**Pattern:** Bluetooth appears "connected" at the BlueZ/D-Bus level (bluetoothctl shows connected) but no `bluez_input.*` node appears in PipeWire. Parec captures pure silence. Equalizer shows flat line.
**Root Cause:** Either `libspa-0.2-bluetooth` is not installed, or WirePlumber lacks `bluez5.roles = [ a2dp_sink ]` config. Without the SPA plugin, WirePlumber cannot register BlueZ5 endpoints, acquire transports, or create PipeWire audio nodes.
**Rule:** The doctor script must check for `device.api = "bluez5"` in `pw-cli list-objects`. If absent, install the SPA plugin and write the WP config. This is the #1 root cause of "BT connected but no audio."

## Default Sink Must Be soundsync-capture
**Pattern:** Bluetooth audio flows to whatever PipeWire's default sink is. If the default sink is the ALSA hardware output (e.g., `alsa_output.pci-*`), Bluetooth audio goes to speakers instead of SoundSync's capture pipeline.
**Rule:** After any service restart, explicitly set the default sink to `soundsync-capture` (or `effect_input.soundsync-eq` if EQ is enabled). The doctor script must enforce this in the repair phase.

## Bluetooth Class of Device Determines Phone Behavior
**Pattern:** With Class `0x0004010c` (Computer), phones see SoundSync as a laptop and may not offer A2DP streaming. With `0x240414` (Audio/Video + Rendering + Loudspeaker), phones see it as a speaker and auto-stream.
**Rule:** The doctor script must verify the *runtime* Class of Device via `bluetoothctl show`, not just the config file. If wrong, fix the config AND use `hciconfig hci0 class 0x240414` for immediate effect without reboot.

## WirePlumber 0.4.x Ignores wireplumber.conf.d/ Files
**Pattern:** WirePlumber 0.5+ uses SPA-JSON `.conf` files in `/etc/wireplumber/wireplumber.conf.d/`. WirePlumber 0.4.x uses Lua files in `/etc/wireplumber/bluetooth.lua.d/`. A `.conf` file written for WP 0.5+ is **silently ignored** by WP 0.4.x — the diagnostic check passes ("config file found with a2dp_sink") but the config is never loaded.
**Root Cause:** The version detection code wrote the correct format, but a previous run (or the old doctor script) wrote the wrong format. The diagnostic only checked "does any file with a2dp_sink exist?" without verifying it matches the WP version.
**Rule:** ALWAYS verify the config format matches the detected WP version. If WP is 0.4.x, only Lua configs in `bluetooth.lua.d/` count. Clean up wrong-format configs from other directories.

## WP 0.4.x Lua: Never Replace bluez_monitor.properties Table
**Pattern:** Writing `bluez_monitor.properties = { ["bluez5.roles"] = "[ a2dp_sink ]" }` in a Lua override file completely replaces the properties table, wiping all defaults from `50-bluez-config.lua` — including the critical `["with-logind"] = true`. Without logind integration, the BlueZ monitor never activates for the user session, and zero bluez5 devices appear in PipeWire.
**Fix:** Use individual property assignment: `bluez_monitor.properties["bluez5.roles"] = "[ a2dp_sink ]"`. This preserves all existing defaults while adding/overriding only the specific properties needed.
**Impact:** This was the actual root cause of "Bluetooth connects but no audio" — the SPA plugin was installed, the config file existed in the right directory, WirePlumber was running, but the BlueZ monitor silently failed to activate because logind integration was wiped.

## "Unknown Transport" = Competing Audio Server
**Pattern:** WirePlumber logs "Properties changed in unknown transport... Multiple sound server instances" when another process (PulseAudio daemon, bluez-alsa) registers competing Bluetooth endpoints with BlueZ. WirePlumber cannot acquire the transport and never creates `bluez_input.*` nodes.
**Rule:** The doctor script must detect and stop PulseAudio (the real daemon, not PipeWire-pulse) and bluez-alsa before restarting audio services. Check with `pgrep -x pulseaudio` and `pgrep -f bluealsa`.

## grep -c Through run_as_user Returns Multiline Text
**Pattern:** `run_as_user "pw-cli list-objects | grep -c something"` returns output with extra newlines when piped through `su -c`. Bash `[[ $var -gt 0 ]]` fails with "syntax error in expression" when var contains `0\n0` instead of `0`.
**Rule:** Always pipe `grep -c` through `tr -dc '0-9'` and add a `${var:-0}` fallback when the result will be used in arithmetic comparisons.

## serde rename_all = "camelCase" Mishandles Acronyms in Field Names
**Pattern:** `#[serde(rename_all = "camelCase")]` on a struct with field `sdp_mline_index` produces JSON key `sdpMlineIndex` (lowercase 'l'). The WebRTC spec and all browsers expect `sdpMLineIndex` (capital 'L' because "MLine" is an abbreviation).
**Impact:** Safari strictly requires at least one of `sdpMid` or `sdpMLineIndex` to be non-null on every ICE candidate. Since the wrong key name means the client never reads the value, Safari drops valid candidates and the WebRTC connection fails entirely. Chrome is more lenient and works anyway if `sdpMid` is present.
**Rule:** Never use `rename_all = "camelCase"` on structs with acronym-containing field names. Use explicit `#[serde(rename = "...")]` on each field to match the exact casing required by the spec.

## RTP Timestamps Must Advance When Frames Are Dropped
**Pattern:** When a broadcast channel receiver reports `Lagged(n)` (n frames were dropped because the receiver was too slow), the RTP timestamp and sequence number must advance by `n * samples_per_frame` and `n` respectively. Without this, the browser's jitter buffer sees timestamps that are behind wall-clock time, causing it to play audio too fast as it tries to "catch up," producing audible stuttering.
**Rule:** Always account for dropped frames in RTP timestamp/sequence tracking. After a lag event, advance both counters to maintain the real-time relationship between timestamps and wall-clock time.

## CPU-Bound Work Must Not Run on Tokio Async Threads
**Pattern:** Opus encoding (`encode_float`) takes 0.5-2ms per 20ms frame. Running this synchronously inside a `tokio::spawn` task blocks the async runtime thread, delaying all other async work on that thread (including other WebRTC sessions, WebSocket messages, and HTTP requests).
**Rule:** Use `tokio::task::spawn_blocking` for CPU-bound audio encoding. Wrap the encoder in `Arc<Mutex>` for cross-thread access. This keeps the async runtime responsive and prevents cascading latency spikes.

## Broadcast Channel Capacity Must Account for Slowest Consumer
**Pattern:** A broadcast channel with capacity 64 (1.28 seconds at 20ms/frame) causes all subscribers to receive `Lagged` errors when ANY single consumer falls behind. The spectrum analyzer doing FFT, HTTP stream encoders, or a briefly delayed WebRTC pump can trigger lag for everyone. This causes frame drops and stuttering even when CPU usage is low.
**Rule:** Size broadcast channels for at least 5 seconds of buffering (256+ frames at 20ms). The cost is minimal (~1.8 MB for 256 × 7680 bytes) but prevents cascade lag from slow consumers.

## parec --latency-msec Must Exceed PipeWire Quantum
**Pattern:** `parec --latency-msec=20` requests a 20ms internal buffer in parec. But PipeWire's quantum (buffer processing size) defaults to 1024 samples at 48 kHz = 21.3ms. If the quantum exceeds parec's buffer, parec cannot hold a full processing cycle, causing buffer underruns that are silent at the system level but produce audible gaps.
**Rule:** Set parec latency to at least 2× the expected PipeWire quantum. 50ms is safe for all common quantum sizes (256, 512, 1024, 2048).

## ALWAYS Bump Version on Every Change
**Pattern:** Deploying code changes without bumping the version number makes it impossible to verify whether a fix is running in production. When a user reports "it's still stuttering," you cannot distinguish between "the fix didn't work" and "the fix isn't deployed."
**Rule:** Bump the patch version (X.Y.Z+1) on EVERY commit that changes behavior. Update ALL 4 locations: Cargo.toml, webui/package.json, webui/src/version.ts, install.sh.
