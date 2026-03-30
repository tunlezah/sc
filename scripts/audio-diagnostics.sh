#!/usr/bin/env bash
# =============================================================================
# SoundSync Audio Pipeline Diagnostics
# =============================================================================
# Run this on the server where SoundSync is running to diagnose audio stutter.
# Usage: bash scripts/audio-diagnostics.sh [--full]
#
# --full: Include extended tracing (strace, perf) — requires root.
# =============================================================================
set -euo pipefail

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
BOLD='\033[1m'
NC='\033[0m'

FULL_MODE=false
[[ "${1:-}" == "--full" ]] && FULL_MODE=true

section() { echo -e "\n${CYAN}${BOLD}═══ $1 ═══${NC}"; }
ok()      { echo -e "  ${GREEN}[OK]${NC} $*"; }
warn()    { echo -e "  ${YELLOW}[WARN]${NC} $*"; }
fail()    { echo -e "  ${RED}[FAIL]${NC} $*"; }
info()    { echo -e "  [INFO] $*"; }

REPORT_DIR="/tmp/soundsync-diag-$(date +%Y%m%d-%H%M%S)"
mkdir -p "$REPORT_DIR"
echo "Diagnostics output: $REPORT_DIR"

# =============================================================================
section "1. PipeWire / PulseAudio Status"
# =============================================================================

if command -v pw-cli &>/dev/null; then
    PW_VERSION=$(pw-cli --version 2>/dev/null | head -1 || echo "unknown")
    info "PipeWire version: $PW_VERSION"
else
    fail "pw-cli not found"
fi

if systemctl --user is-active pipewire &>/dev/null; then
    ok "pipewire.service is active"
else
    # Try without --user for system services
    if systemctl is-active pipewire &>/dev/null; then
        ok "pipewire.service is active (system)"
    else
        fail "pipewire.service is NOT active"
    fi
fi

if systemctl --user is-active pipewire-pulse &>/dev/null; then
    ok "pipewire-pulse.service is active"
else
    if systemctl is-active pipewire-pulse &>/dev/null; then
        ok "pipewire-pulse.service is active (system)"
    else
        warn "pipewire-pulse.service is NOT active"
    fi
fi

if systemctl --user is-active wireplumber &>/dev/null; then
    ok "wireplumber.service is active"
else
    if systemctl is-active wireplumber &>/dev/null; then
        ok "wireplumber.service is active (system)"
    else
        warn "wireplumber.service is NOT active"
    fi
fi

# =============================================================================
section "2. PipeWire Graph Quantum/Buffer Settings (CRITICAL)"
# =============================================================================

# These are the most likely cause of stutter across all outputs.
# PipeWire's "quantum" is the buffer size in samples. Too small = xruns.
# Default quantum is typically 1024 at 48kHz (~21ms).

if command -v pw-metadata &>/dev/null; then
    info "Current PipeWire clock settings:"
    pw-metadata -n settings 2>/dev/null | tee "$REPORT_DIR/pw-metadata.txt" || true
    echo ""

    # Extract key values
    QUANTUM=$(pw-metadata -n settings 2>/dev/null | grep "clock.force-quantum" | grep -oP 'value:"[^"]*"' | cut -d'"' -f2 || echo "default")
    RATE=$(pw-metadata -n settings 2>/dev/null | grep "clock.force-rate" | grep -oP 'value:"[^"]*"' | cut -d'"' -f2 || echo "default")
    MIN_Q=$(pw-metadata -n settings 2>/dev/null | grep "clock.min-quantum" | grep -oP 'value:"[^"]*"' | cut -d'"' -f2 || echo "default")
    MAX_Q=$(pw-metadata -n settings 2>/dev/null | grep "clock.max-quantum" | grep -oP 'value:"[^"]*"' | cut -d'"' -f2 || echo "default")

    info "  clock.force-quantum: ${QUANTUM}"
    info "  clock.force-rate:    ${RATE}"
    info "  clock.min-quantum:   ${MIN_Q}"
    info "  clock.max-quantum:   ${MAX_Q}"

    if [[ "$QUANTUM" != "default" && "$QUANTUM" != "" ]]; then
        Q_VAL=$(echo "$QUANTUM" | tr -dc '0-9')
        if [[ -n "$Q_VAL" && "$Q_VAL" -lt 256 ]]; then
            warn "Quantum is very low ($Q_VAL) — likely causing xruns/stutter!"
        elif [[ -n "$Q_VAL" && "$Q_VAL" -ge 1024 ]]; then
            ok "Quantum ($Q_VAL) is adequate for stability"
        fi
    fi
else
    warn "pw-metadata not found — cannot check quantum settings"
fi

# Check PipeWire config files for quantum overrides
info ""
info "Checking PipeWire config files for quantum/buffer overrides..."
for f in \
    /etc/pipewire/pipewire.conf \
    /etc/pipewire/pipewire.conf.d/*.conf \
    ~/.config/pipewire/pipewire.conf \
    ~/.config/pipewire/pipewire.conf.d/*.conf \
    /usr/share/pipewire/pipewire.conf; do
    if [[ -f "$f" ]]; then
        if grep -q "quantum" "$f" 2>/dev/null; then
            warn "Quantum override found in: $f"
            grep -n "quantum" "$f" | head -5
        fi
    fi
done 2>/dev/null || true

# =============================================================================
section "3. PipeWire Xrun Detection (Buffer Underruns)"
# =============================================================================

info "Checking for xruns in PipeWire..."
if command -v pw-top &>/dev/null; then
    info "Running pw-top for 5 seconds to capture xrun counts..."
    timeout 5 pw-top -b 2>/dev/null | head -50 > "$REPORT_DIR/pw-top.txt" || true
    if [[ -s "$REPORT_DIR/pw-top.txt" ]]; then
        # Look for xrun indicators
        XRUNS=$(grep -c "XRUN\|xrun\|ERR" "$REPORT_DIR/pw-top.txt" 2>/dev/null || echo "0")
        if [[ "$XRUNS" -gt 0 ]]; then
            fail "Detected $XRUNS xrun events in pw-top output!"
            cat "$REPORT_DIR/pw-top.txt"
        else
            ok "No xruns detected in pw-top (5s sample)"
        fi
    fi
fi

# Also check journal for xrun messages
if command -v journalctl &>/dev/null; then
    XRUN_JOURNAL=$(journalctl --user -u pipewire --since "10 minutes ago" 2>/dev/null | grep -ci "xrun\|underrun\|overrun" || echo "0")
    if [[ "$XRUN_JOURNAL" -gt 0 ]]; then
        fail "Found $XRUN_JOURNAL xrun/underrun mentions in pipewire journal (last 10 min)"
        journalctl --user -u pipewire --since "10 minutes ago" 2>/dev/null | grep -i "xrun\|underrun\|overrun" | tail -10
    else
        ok "No xrun messages in pipewire journal (last 10 min)"
    fi
fi

# =============================================================================
section "4. Audio Graph Nodes & Routing"
# =============================================================================

if command -v pw-cli &>/dev/null; then
    info "PipeWire node list:"
    pw-cli list-objects 2>/dev/null | tee "$REPORT_DIR/pw-objects.txt" | head -40
    echo "  ... (full output in $REPORT_DIR/pw-objects.txt)"
fi

if command -v pactl &>/dev/null; then
    info ""
    info "PulseAudio sinks:"
    pactl list short sinks 2>/dev/null | tee "$REPORT_DIR/pa-sinks.txt"

    info ""
    info "PulseAudio sources:"
    pactl list short sources 2>/dev/null | tee "$REPORT_DIR/pa-sources.txt"

    info ""
    info "PulseAudio modules:"
    pactl list short modules 2>/dev/null | tee "$REPORT_DIR/pa-modules.txt"

    info ""
    info "Default sink:"
    DEFAULT_SINK=$(pactl get-default-sink 2>/dev/null || echo "unknown")
    info "  $DEFAULT_SINK"

    # Verify routing
    if echo "$DEFAULT_SINK" | grep -q "soundsync"; then
        ok "Default sink is routed to SoundSync"
    elif echo "$DEFAULT_SINK" | grep -q "effect_input"; then
        ok "Default sink is routed through EQ to SoundSync"
    else
        warn "Default sink ($DEFAULT_SINK) may not be routed to SoundSync"
    fi
fi

# =============================================================================
section "5. Null Sink & Filter-Chain Health"
# =============================================================================

if command -v pactl &>/dev/null; then
    if pactl list short sinks 2>/dev/null | grep -q "soundsync-capture"; then
        ok "soundsync-capture null sink exists"
    else
        fail "soundsync-capture null sink NOT found — audio capture will fail"
    fi

    if pactl list short sinks 2>/dev/null | grep -q "effect_input.soundsync-eq"; then
        ok "EQ filter-chain sink (effect_input.soundsync-eq) exists"
    else
        warn "EQ filter-chain sink not found — EQ may be disabled or failed to start"
    fi
fi

# Check if filter-chain process is running
if pgrep -f "pipewire.*filter-chain\|pipewire-filter-chain" &>/dev/null; then
    ok "Filter-chain process is running"
    pgrep -af "pipewire.*filter-chain\|pipewire-filter-chain" | head -2
else
    warn "No filter-chain process found — EQ is not active"
fi

# =============================================================================
section "6. Capture Process (parec/pw-cat)"
# =============================================================================

if pgrep -f "parec.*soundsync\|pw-cat.*soundsync" &>/dev/null; then
    ok "Audio capture process is running:"
    pgrep -af "parec.*soundsync\|pw-cat.*soundsync" | head -2
else
    if pgrep -f "parec\|pw-cat" &>/dev/null; then
        warn "A parec/pw-cat process is running but may not target SoundSync:"
        pgrep -af "parec\|pw-cat" | head -2
    else
        fail "No audio capture process (parec/pw-cat) running!"
    fi
fi

# =============================================================================
section "7. FFmpeg (AAC Encoder) Health"
# =============================================================================

if pgrep -f "ffmpeg.*aac\|ffmpeg.*adts" &>/dev/null; then
    ok "FFmpeg AAC encoder process is running"
    pgrep -af "ffmpeg.*aac\|ffmpeg.*adts" | head -2
else
    info "No FFmpeg AAC encoder process detected (may start on-demand per stream request)"
fi

if command -v ffmpeg &>/dev/null; then
    ok "ffmpeg is available: $(ffmpeg -version 2>/dev/null | head -1)"
else
    warn "ffmpeg not installed — AAC streaming will fall back to MP3"
fi

# =============================================================================
section "8. Bluetooth A2DP Status"
# =============================================================================

if command -v bluetoothctl &>/dev/null; then
    info "Connected Bluetooth devices:"
    bluetoothctl devices Connected 2>/dev/null | tee "$REPORT_DIR/bt-connected.txt" || \
        bluetoothctl info 2>/dev/null | grep -A5 "Connected:" | tee "$REPORT_DIR/bt-connected.txt" || true
fi

# Check for bluez source nodes in PipeWire
if command -v pw-cli &>/dev/null; then
    BT_NODES=$(pw-cli list-objects 2>/dev/null | grep -c "bluez_input\|bluez_source" || echo "0")
    if [[ "$BT_NODES" -gt 0 ]]; then
        ok "Found $BT_NODES Bluetooth audio node(s) in PipeWire"
        pw-cli list-objects 2>/dev/null | grep "bluez_input\|bluez_source"
    else
        info "No Bluetooth audio nodes in PipeWire (no BT device streaming)"
    fi
fi

# Check Bluetooth codec in use
if command -v pw-dump &>/dev/null; then
    info ""
    info "Bluetooth codec info (from pw-dump):"
    pw-dump 2>/dev/null | python3 -c "
import json, sys
try:
    data = json.load(sys.stdin)
    for obj in data:
        props = obj.get('info', {}).get('props', {})
        if 'bluez' in props.get('node.name', ''):
            print(f\"  Node: {props.get('node.name', 'N/A')}\")
            print(f\"  Codec: {props.get('api.bluez5.codec', 'N/A')}\")
            print(f\"  Rate: {props.get('audio.rate', 'N/A')}\")
            print(f\"  Format: {props.get('audio.format', 'N/A')}\")
            print(f\"  Channels: {props.get('audio.channels', 'N/A')}\")
except:
    pass
" 2>/dev/null || info "  (could not parse pw-dump output)"
fi

# =============================================================================
section "9. SoundSync Process Status"
# =============================================================================

if pgrep -f "soundsync" &>/dev/null; then
    ok "SoundSync process is running"
    pgrep -af "soundsync" | grep -v "diag" | head -3

    SS_PID=$(pgrep -f "soundsync" | grep -v "$$" | head -1)
    if [[ -n "$SS_PID" ]]; then
        info "PID: $SS_PID"
        info "Threads: $(ls /proc/$SS_PID/task 2>/dev/null | wc -l)"
        info "Open FDs: $(ls /proc/$SS_PID/fd 2>/dev/null | wc -l)"
        info "Memory (RSS): $(awk '/VmRSS/{print $2" "$3}' /proc/$SS_PID/status 2>/dev/null || echo "N/A")"
        info "CPU time: $(awk '{printf "user=%.2fs sys=%.2fs", $14/100, $15/100}' /proc/$SS_PID/stat 2>/dev/null || echo "N/A")"
    fi
else
    fail "SoundSync process is NOT running"
fi

# =============================================================================
section "10. System I/O & Scheduling"
# =============================================================================

info "CPU governor:"
for gov in /sys/devices/system/cpu/cpu*/cpufreq/scaling_governor; do
    if [[ -f "$gov" ]]; then
        GOV=$(cat "$gov")
        if [[ "$GOV" == "powersave" ]]; then
            warn "CPU governor: $GOV — may cause latency spikes on frequency transitions!"
        else
            ok "CPU governor: $GOV"
        fi
        break
    fi
done 2>/dev/null || info "  Cannot read CPU governor"

info ""
info "Kernel scheduling parameters:"
if [[ -f /proc/sys/kernel/sched_latency_ns ]]; then
    info "  sched_latency_ns:      $(cat /proc/sys/kernel/sched_latency_ns)"
    info "  sched_min_granularity: $(cat /proc/sys/kernel/sched_min_granularity_ns 2>/dev/null || echo 'N/A')"
fi

info ""
info "Timer resolution:"
if [[ -f /proc/timer_list ]]; then
    grep "resolution" /proc/timer_list 2>/dev/null | head -1 || true
fi

info ""
info "Real-time priorities (SoundSync process):"
if [[ -n "${SS_PID:-}" ]]; then
    for tid in /proc/$SS_PID/task/*; do
        TID=$(basename "$tid")
        SCHED=$(chrt -p "$TID" 2>/dev/null || echo "N/A")
        COMM=$(cat "$tid/comm" 2>/dev/null || echo "?")
        if echo "$SCHED" | grep -q "SCHED_RR\|SCHED_FIFO"; then
            info "  Thread $TID ($COMM): $SCHED"
        fi
    done
fi

info ""
info "Memory pressure:"
if [[ -f /proc/meminfo ]]; then
    awk '/MemTotal|MemAvailable|SwapTotal|SwapFree/' /proc/meminfo
fi

info ""
info "Swap activity (may cause latency spikes):"
if command -v vmstat &>/dev/null; then
    vmstat 1 3 2>/dev/null | tail -3 | tee "$REPORT_DIR/vmstat.txt"
fi

# =============================================================================
section "11. Network Diagnostics"
# =============================================================================

info "Active network listeners on SoundSync port:"
ss -tlnp | grep ":8080\|soundsync" 2>/dev/null || info "  (no listeners found on 8080)"

info ""
info "TCP socket buffer sizes:"
info "  tcp_rmem: $(cat /proc/sys/net/ipv4/tcp_rmem 2>/dev/null || echo 'N/A')"
info "  tcp_wmem: $(cat /proc/sys/net/ipv4/tcp_wmem 2>/dev/null || echo 'N/A')"

info ""
info "TCP Nagle's algorithm (tcp_nodelay not system-wide, checked per socket):"
info "  Note: Nagle is enabled by default on TCP sockets. Chunked HTTP streams"
info "  may experience delayed sends when small chunks are produced."

# =============================================================================
section "12. WirePlumber Bluetooth Configuration"
# =============================================================================

info "Checking WirePlumber Bluetooth config..."

# Check WirePlumber version
if command -v wireplumber &>/dev/null; then
    WP_VER=$(wireplumber --version 2>/dev/null | head -1 || echo "unknown")
    info "WirePlumber version: $WP_VER"
fi

# Check for A2DP sink role config
for cfg_dir in \
    /etc/wireplumber/bluetooth.lua.d \
    /etc/wireplumber/wireplumber.conf.d \
    ~/.config/wireplumber; do
    if [[ -d "$cfg_dir" ]]; then
        info "Config dir exists: $cfg_dir"
        ls -la "$cfg_dir"/ 2>/dev/null | head -10
        for f in "$cfg_dir"/*.{lua,conf} 2>/dev/null; do
            if [[ -f "$f" ]]; then
                info "  Content of $f:"
                cat "$f" | head -20
            fi
        done
    fi
done 2>/dev/null || true

# =============================================================================
section "13. Timing Test: Capture Latency Probe"
# =============================================================================

info "Measuring PCM capture timing variance..."
info "(Captures 100 frames from parec and measures inter-frame timing)"

if command -v parec &>/dev/null && pactl list short sinks 2>/dev/null | grep -q "soundsync-capture"; then
    python3 -c "
import subprocess, time, statistics

# Read 100 frames (each 7680 bytes = 20ms of 48kHz stereo f32)
FRAME_BYTES = 7680
NUM_FRAMES = 100
EXPECTED_MS = 20.0

proc = subprocess.Popen(
    ['parec', '--raw', '--format=float32', '--channels=2', '--rate=48000',
     '--device=soundsync-capture.monitor', '--latency-msec=20'],
    stdout=subprocess.PIPE, stderr=subprocess.DEVNULL
)

intervals = []
last_time = time.monotonic()

for i in range(NUM_FRAMES):
    data = proc.stdout.read(FRAME_BYTES)
    if len(data) < FRAME_BYTES:
        print(f'  Short read at frame {i}: {len(data)} bytes')
        break
    now = time.monotonic()
    if i > 0:
        intervals.append((now - last_time) * 1000)
    last_time = now

proc.terminate()
proc.wait()

if len(intervals) > 10:
    mean = statistics.mean(intervals)
    stdev = statistics.stdev(intervals)
    jitter = max(intervals) - min(intervals)
    p95 = sorted(intervals)[int(len(intervals) * 0.95)]
    p99 = sorted(intervals)[int(len(intervals) * 0.99)]

    print(f'  Frames captured: {len(intervals) + 1}')
    print(f'  Expected interval: {EXPECTED_MS:.1f} ms')
    print(f'  Mean interval:     {mean:.2f} ms')
    print(f'  Std deviation:     {stdev:.2f} ms')
    print(f'  Jitter (max-min):  {jitter:.2f} ms')
    print(f'  P95:               {p95:.2f} ms')
    print(f'  P99:               {p99:.2f} ms')
    print(f'  Min:               {min(intervals):.2f} ms')
    print(f'  Max:               {max(intervals):.2f} ms')

    outliers = [x for x in intervals if abs(x - mean) > 3 * stdev]
    if outliers:
        print(f'  OUTLIERS (>3 sigma): {len(outliers)} frames')
        for o in outliers[:5]:
            print(f'    {o:.2f} ms')

    if stdev > 5.0:
        print(f'  WARNING: High timing variance (stdev={stdev:.2f}ms) — likely cause of stutter!')
    elif jitter > 40.0:
        print(f'  WARNING: High jitter ({jitter:.2f}ms) — occasional stutter expected!')
    else:
        print(f'  Timing looks healthy.')
else:
    print('  Not enough data captured for analysis.')
" 2>&1 | tee "$REPORT_DIR/capture-timing.txt" || warn "Timing test failed (is audio playing?)"
else
    info "  Skipped — parec or soundsync-capture not available"
fi

# =============================================================================
section "14. Broadcast Channel Saturation Test"
# =============================================================================

info "Checking SoundSync logs for broadcast lag indicators..."

if command -v journalctl &>/dev/null; then
    LAG_COUNT=$(journalctl --user -u soundsync --since "30 minutes ago" 2>/dev/null | grep -ci "lagged\|lag" || echo "0")
    if [[ "$LAG_COUNT" -gt 0 ]]; then
        warn "Found $LAG_COUNT broadcast lag messages in the last 30 minutes!"
        journalctl --user -u soundsync --since "30 minutes ago" 2>/dev/null | grep -i "lagged\|lag" | tail -20
    else
        # Also check system-level journal
        LAG_COUNT=$(journalctl -u soundsync --since "30 minutes ago" 2>/dev/null | grep -ci "lagged\|lag" || echo "0")
        if [[ "$LAG_COUNT" -gt 0 ]]; then
            warn "Found $LAG_COUNT broadcast lag messages (system journal, last 30 min)"
        else
            ok "No broadcast lag messages found in logs"
        fi
    fi

    # Check for general errors
    ERR_COUNT=$(journalctl -u soundsync --since "30 minutes ago" 2>/dev/null | grep -ci "error\|panic\|WARN" || echo "0")
    info "Error/Warning count in last 30 min: $ERR_COUNT"
    if [[ "$ERR_COUNT" -gt 10 ]]; then
        warn "High error/warning count. Recent messages:"
        journalctl -u soundsync --since "30 minutes ago" 2>/dev/null | grep -i "error\|panic\|WARN" | tail -10
    fi
fi

# =============================================================================
section "15. Extended Tracing (--full mode only)"
# =============================================================================

if $FULL_MODE; then
    if [[ -n "${SS_PID:-}" ]]; then
        info "Running strace on SoundSync for 5 seconds..."
        timeout 5 strace -c -p "$SS_PID" 2>"$REPORT_DIR/strace-summary.txt" || true
        if [[ -s "$REPORT_DIR/strace-summary.txt" ]]; then
            cat "$REPORT_DIR/strace-summary.txt"
        fi

        info ""
        info "Running perf stat for 5 seconds..."
        timeout 5 perf stat -p "$SS_PID" 2>"$REPORT_DIR/perf-stat.txt" || true
        if [[ -s "$REPORT_DIR/perf-stat.txt" ]]; then
            cat "$REPORT_DIR/perf-stat.txt"
        fi
    else
        warn "SoundSync PID not found — skipping strace/perf"
    fi

    info ""
    info "Checking for RT scheduling capability..."
    if command -v ulimit &>/dev/null; then
        info "  rtprio soft: $(ulimit -Sr 2>/dev/null || echo 'N/A')"
        info "  rtprio hard: $(ulimit -Hr 2>/dev/null || echo 'N/A')"
        info "  memlock:     $(ulimit -Sl 2>/dev/null || echo 'N/A')"
    fi
else
    info "Extended tracing skipped. Run with --full for strace/perf analysis."
fi

# =============================================================================
section "Summary"
# =============================================================================

echo ""
echo "Diagnostics complete. Full output saved to: $REPORT_DIR"
echo ""
echo "Key files:"
echo "  $REPORT_DIR/pw-metadata.txt     - PipeWire quantum/clock settings"
echo "  $REPORT_DIR/pw-top.txt          - PipeWire node xrun status"
echo "  $REPORT_DIR/capture-timing.txt  - PCM capture timing analysis"
echo "  $REPORT_DIR/vmstat.txt          - Memory/swap activity"
echo ""
echo "To share diagnostics, run:"
echo "  tar czf soundsync-diag.tar.gz -C /tmp $(basename $REPORT_DIR)"
echo ""
