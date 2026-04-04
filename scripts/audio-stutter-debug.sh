#!/usr/bin/env bash
# audio-stutter-debug.sh — Live audio stuttering diagnostic tool
#
# Run this ON THE SERVER while audio is playing to capture diagnostic data.
# It collects process info, PipeWire stats, buffer health, error counts,
# and timing data that help identify the root cause of audio stuttering.
#
# Usage:
#   sudo -u soundsync bash scripts/audio-stutter-debug.sh
#   # or as whatever user runs SoundSync
#
# The script runs for DURATION seconds (default 30) and outputs a report.
# Pipe to a file for sharing: bash scripts/audio-stutter-debug.sh > diag.txt 2>&1

set -euo pipefail

DURATION="${1:-30}"
INTERVAL=2  # sample every 2 seconds
SAMPLES=$((DURATION / INTERVAL))

echo "======================================================================"
echo " SoundSync Audio Stutter Diagnostic"
echo " $(date '+%Y-%m-%d %H:%M:%S %Z')"
echo " Duration: ${DURATION}s (sampling every ${INTERVAL}s)"
echo "======================================================================"

# ── Section 1: System Overview ──────────────────────────────────────────
echo ""
echo "══════════════════════════════════════════════════════════════════════"
echo " 1. SYSTEM OVERVIEW"
echo "══════════════════════════════════════════════════════════════════════"

echo ""
echo "--- Kernel & OS ---"
uname -a
cat /etc/os-release 2>/dev/null | grep -E "^(NAME|VERSION)=" || true

echo ""
echo "--- CPU Info ---"
nproc 2>/dev/null && echo "cores"
cat /proc/cpuinfo 2>/dev/null | grep "model name" | head -1 || true

echo ""
echo "--- Kernel Timer Hz ---"
# Low CONFIG_HZ (100/250) causes coarser scheduling → more jitter
grep -i "config_hz" /boot/config-$(uname -r) 2>/dev/null || echo "Cannot read kernel config"

echo ""
echo "--- CPU Frequency Governor ---"
# 'powersave' governor causes frequency scaling → latency spikes
for f in /sys/devices/system/cpu/cpu*/cpufreq/scaling_governor; do
    [ -f "$f" ] && cat "$f" && break
done 2>/dev/null || echo "No cpufreq info available"

echo ""
echo "--- Memory ---"
free -h | head -3

echo ""
echo "--- Swap Usage ---"
swapon --show 2>/dev/null || echo "No swap configured"

echo ""
echo "--- System Load ---"
uptime

# ── Section 2: Audio Process Inventory ──────────────────────────────────
echo ""
echo "══════════════════════════════════════════════════════════════════════"
echo " 2. AUDIO PROCESS INVENTORY (CRITICAL: check for duplicates)"
echo "══════════════════════════════════════════════════════════════════════"

echo ""
echo "--- SoundSync processes ---"
ps aux | grep -E '[s]oundsync' || echo "No soundsync processes found"

echo ""
echo "--- Audio capture processes (parec/pw-cat) ---"
echo "  *** Multiple parec/pw-cat = DUPLICATE CAPTURE = STUTTERING ***"
ps aux | grep -E '[p]arec|[p]w-cat' || echo "No capture processes"

echo ""
echo "--- FFmpeg processes (AAC encoding) ---"
echo "  *** Multiple ffmpeg = DUPLICATE ENCODING = STUTTERING ***"
ps aux | grep -E '[f]fmpeg' || echo "No ffmpeg processes"

echo ""
echo "--- PipeWire filter-chain processes ---"
echo "  *** Multiple filter-chains = DUPLICATE EQ = STUTTERING ***"
ps aux | grep -E '[p]ipewire.*filter|[f]ilter.chain' || echo "No filter-chain processes"

echo ""
echo "--- PipeWire loopback processes ---"
ps aux | grep -E '[p]w-loopback' || echo "No pw-loopback processes"

echo ""
echo "--- Competing audio servers (PulseAudio daemon, bluealsa) ---"
echo "  *** These steal Bluetooth transports from WirePlumber ***"
ps aux | grep -E '[p]ulseaudio|[b]luealsa' || echo "None found (good)"

echo ""
echo "--- All audio-related processes ---"
ps aux | grep -iE 'pipewire|wireplumber|pulse|parec|pw-cat|pw-loopback|ffmpeg|soundsync|bluealsa' | grep -v grep || echo "None"

# ── Section 3: PipeWire State ───────────────────────────────────────────
echo ""
echo "══════════════════════════════════════════════════════════════════════"
echo " 3. PIPEWIRE STATE"
echo "══════════════════════════════════════════════════════════════════════"

echo ""
echo "--- PipeWire service status ---"
systemctl --user status pipewire --no-pager 2>&1 | head -5 || echo "Cannot check (wrong user or not systemd)"

echo ""
echo "--- WirePlumber service status ---"
systemctl --user status wireplumber --no-pager 2>&1 | head -5 || echo "Cannot check"

echo ""
echo "--- PipeWire settings (quantum/rate) ---"
# Quantum = buffer size in samples. 1024@48000 = 21.3ms, 512@48000 = 10.7ms
pw-metadata -n settings 2>/dev/null || echo "pw-metadata not available"

echo ""
echo "--- Default sink ---"
pactl get-default-sink 2>/dev/null || echo "pactl not available"

echo ""
echo "--- All sinks ---"
pactl list short sinks 2>/dev/null || echo "pactl not available"

echo ""
echo "--- All sources ---"
pactl list short sources 2>/dev/null || echo "pactl not available"

echo ""
echo "--- PipeWire links (audio routing) ---"
pw-link -l 2>/dev/null | head -60 || echo "pw-link not available"

echo ""
echo "--- PipeWire node details (buffer info) ---"
pw-cli list-objects 2>/dev/null | grep -A5 -E "soundsync|bluez|null-sink" | head -60 || echo "pw-cli not available"

# ── Section 4: PipeWire Statistics & Errors ─────────────────────────────
echo ""
echo "══════════════════════════════════════════════════════════════════════"
echo " 4. PIPEWIRE STATISTICS & BUFFER HEALTH"
echo "══════════════════════════════════════════════════════════════════════"

echo ""
echo "--- pw-top snapshot (buffer underruns/overruns) ---"
echo "  QUANT = quantum size, RATE = sample rate"
echo "  WAIT/BUSY = processing timing, XRUN = buffer under/overrun count"
echo "  *** High XRUN count = STUTTERING ROOT CAUSE ***"
if command -v pw-top &>/dev/null; then
    # pw-top in batch mode: capture one snapshot
    timeout 3 pw-top -b 2>/dev/null | head -40 || echo "pw-top batch mode not available"
else
    echo "pw-top not installed"
fi

echo ""
echo "--- PipeWire profiler data ---"
if command -v pw-cli &>/dev/null; then
    pw-cli info 0 2>/dev/null | head -20 || true
fi

# ── Section 5: Process Scheduling & Priority ────────────────────────────
echo ""
echo "══════════════════════════════════════════════════════════════════════"
echo " 5. PROCESS SCHEDULING & PRIORITY"
echo "══════════════════════════════════════════════════════════════════════"

echo ""
echo "--- Real-time scheduling capabilities ---"
echo "  *** Audio processes SHOULD run at SCHED_FIFO or SCHED_RR ***"
echo "  *** SCHED_OTHER (0) = normal priority = vulnerable to preemption ***"

for proc in parec pw-cat soundsync ffmpeg pipewire wireplumber; do
    pids=$(pgrep -x "$proc" 2>/dev/null || pgrep -f "$proc" 2>/dev/null | head -3 || true)
    if [ -n "$pids" ]; then
        for pid in $pids; do
            sched_info=$(chrt -p "$pid" 2>/dev/null || echo "cannot read")
            nice_val=$(ps -o ni= -p "$pid" 2>/dev/null || echo "?")
            echo "  $proc (PID $pid): $sched_info  nice=$nice_val"
        done
    fi
done

echo ""
echo "--- rtkit-daemon (realtime privilege escalation) ---"
ps aux | grep -E '[r]tkit' || echo "rtkit-daemon not running (real-time scheduling unavailable)"

echo ""
echo "--- Audio group membership ---"
id 2>/dev/null || true
groups 2>/dev/null | grep -oE "(audio|pipewire|rtkit)" || echo "User not in audio/pipewire/rtkit groups"

echo ""
echo "--- Real-time limits (ulimit) ---"
echo "  RTPRIO (max real-time priority): $(ulimit -r 2>/dev/null || echo 'unknown')"
echo "  MEMLOCK (max locked memory KB): $(ulimit -l 2>/dev/null || echo 'unknown')"

# ── Section 6: Network (WebRTC/UDP) ────────────────────────────────────
echo ""
echo "══════════════════════════════════════════════════════════════════════"
echo " 6. NETWORK (WebRTC/UDP)"
echo "══════════════════════════════════════════════════════════════════════"

echo ""
echo "--- UDP send/receive buffer sizes ---"
echo "  *** Small buffers cause packet drops → audio stuttering ***"
echo "  wmem_default (send): $(cat /proc/sys/net/core/wmem_default 2>/dev/null || echo '?') bytes"
echo "  rmem_default (recv): $(cat /proc/sys/net/core/rmem_default 2>/dev/null || echo '?') bytes"
echo "  wmem_max (send max): $(cat /proc/sys/net/core/wmem_max 2>/dev/null || echo '?') bytes"
echo "  rmem_max (recv max): $(cat /proc/sys/net/core/rmem_max 2>/dev/null || echo '?') bytes"

echo ""
echo "--- Network interface power management ---"
for iface in $(ls /sys/class/net/ 2>/dev/null | grep -v lo); do
    pm=$(cat "/sys/class/net/$iface/device/power/control" 2>/dev/null || echo "N/A")
    echo "  $iface: power_control=$pm"
done

echo ""
echo "--- UDP socket statistics ---"
ss -u -a -n 2>/dev/null | head -20 || echo "ss not available"

# ── Section 7: SoundSync Logs ───────────────────────────────────────────
echo ""
echo "══════════════════════════════════════════════════════════════════════"
echo " 7. SOUNDSYNC RECENT LOGS (last 100 lines)"
echo "══════════════════════════════════════════════════════════════════════"

echo ""
echo "--- Filtering for: lag, stutter, error, xrun, underrun, overrun ---"
journalctl -u soundsync --no-pager -n 200 2>/dev/null | grep -iE "lag|stutter|error|xrun|underrun|overrun|warn|drop|fail|panic" | tail -50 || echo "No matching log entries"

echo ""
echo "--- Last 50 log lines ---"
journalctl -u soundsync --no-pager -n 50 2>/dev/null || echo "journalctl not available"

# ── Section 8: Live Monitoring ──────────────────────────────────────────
echo ""
echo "══════════════════════════════════════════════════════════════════════"
echo " 8. LIVE MONITORING (${DURATION}s capture)"
echo "══════════════════════════════════════════════════════════════════════"
echo ""
echo "Capturing ${SAMPLES} samples every ${INTERVAL}s..."
echo "Watch for: CPU spikes, process count changes, new xruns"
echo ""

printf "%-6s %-8s %-8s %-10s %-10s %-8s %-8s %-30s\n" \
    "TIME" "CPU%" "MEM_MB" "PAREC_CNT" "FFMPEG_CNT" "LOAD_1m" "XRUNS" "NOTES"
printf "%-6s %-8s %-8s %-10s %-10s %-8s %-8s %-30s\n" \
    "------" "--------" "--------" "----------" "----------" "--------" "--------" "------------------------------"

PREV_PAREC_CNT=0
PREV_FFMPEG_CNT=0

for i in $(seq 1 "$SAMPLES"); do
    TS=$(date '+%H:%M:%S')

    # CPU usage (1-second sample)
    CPU=$(top -bn1 -d0.5 2>/dev/null | grep "Cpu(s)" | awk '{print $2+$4}' || echo "?")

    # Memory usage
    MEM=$(free -m | awk '/Mem:/{print $3}')

    # Process counts (CRITICAL: should be exactly 1 each)
    PAREC_CNT=$(pgrep -c -f "parec|pw-cat" 2>/dev/null || echo "0")
    FFMPEG_CNT=$(pgrep -c -f "ffmpeg" 2>/dev/null || echo "0")

    # System load
    LOAD=$(awk '{print $1}' /proc/loadavg 2>/dev/null || echo "?")

    # PipeWire xruns (try to get from pw-top batch mode)
    XRUNS=$(timeout 1 pw-top -b 2>/dev/null | grep -oE "XRUN=[0-9]+" | head -1 || echo "N/A")
    [ -z "$XRUNS" ] && XRUNS="N/A"

    # Detect anomalies
    NOTES=""
    if [ "$PAREC_CNT" -gt 1 ] 2>/dev/null; then
        NOTES="DUPLICATE PAREC!"
    fi
    if [ "$FFMPEG_CNT" -gt 1 ] 2>/dev/null; then
        NOTES="${NOTES} DUPLICATE FFMPEG!"
    fi
    if [ "$PAREC_CNT" != "$PREV_PAREC_CNT" ] && [ "$i" -gt 1 ] 2>/dev/null; then
        NOTES="${NOTES} parec_count_changed!"
    fi

    printf "%-6s %-8s %-8s %-10s %-10s %-8s %-8s %-30s\n" \
        "$TS" "$CPU" "${MEM}M" "$PAREC_CNT" "$FFMPEG_CNT" "$LOAD" "$XRUNS" "$NOTES"

    PREV_PAREC_CNT="$PAREC_CNT"
    PREV_FFMPEG_CNT="$FFMPEG_CNT"

    [ "$i" -lt "$SAMPLES" ] && sleep "$INTERVAL"
done

# ── Section 9: Final Snapshot ───────────────────────────────────────────
echo ""
echo "══════════════════════════════════════════════════════════════════════"
echo " 9. FINAL SNAPSHOT"
echo "══════════════════════════════════════════════════════════════════════"

echo ""
echo "--- Process tree (soundsync and children) ---"
SOUNDSYNC_PID=$(pgrep -x soundsync 2>/dev/null | head -1 || true)
if [ -n "$SOUNDSYNC_PID" ]; then
    pstree -p "$SOUNDSYNC_PID" 2>/dev/null || ps --forest -p "$SOUNDSYNC_PID" 2>/dev/null || true
else
    echo "SoundSync process not found"
fi

echo ""
echo "--- Open file descriptors (soundsync) ---"
if [ -n "$SOUNDSYNC_PID" ]; then
    ls /proc/"$SOUNDSYNC_PID"/fd 2>/dev/null | wc -l || true
    echo "file descriptors open"
fi

echo ""
echo "--- Recent dmesg audio/USB errors ---"
dmesg --ctime 2>/dev/null | tail -100 | grep -iE "xrun|usb|audio|snd|bluetooth|error" | tail -10 || echo "No relevant dmesg entries"

echo ""
echo "======================================================================"
echo " Diagnostic complete at $(date '+%Y-%m-%d %H:%M:%S %Z')"
echo ""
echo " KEY THINGS TO CHECK:"
echo "  1. PAREC_CNT/FFMPEG_CNT should be exactly 1 (not 0, not >1)"
echo "  2. XRUN count should be 0 or very low"
echo "  3. CPU governor should be 'performance' not 'powersave'"
echo "  4. rtkit-daemon should be running"
echo "  5. No 'DUPLICATE' warnings in the live monitoring"
echo "  6. No competing audio servers (pulseaudio, bluealsa)"
echo "  7. PipeWire quantum should be >= 1024 for stability"
echo "======================================================================"
