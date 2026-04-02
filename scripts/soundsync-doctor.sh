#!/usr/bin/env bash
# =============================================================================
# soundsync-doctor.sh — Diagnose and repair SoundSync audio system
# =============================================================================
# Usage: sudo bash scripts/soundsync-doctor.sh [--diagnose-only] [--json]
#
# --diagnose-only   Run checks without attempting repairs
# --json            Output machine-readable JSON summary at end
# =============================================================================
set -uo pipefail
# NOTE: We use set -u but NOT set -e — we handle errors per-command.

# ═══════════════════════════════════════════════════════════════════════════════
# SECTION A: SETUP & HELPERS
# ═══════════════════════════════════════════════════════════════════════════════

# Require root for repair operations
if [[ $EUID -ne 0 ]] && [[ "${1:-}" != "--diagnose-only" ]]; then
    echo "This script must be run as root for repairs. Use: sudo $0 $*"
    echo "Or run with --diagnose-only to skip repairs."
    exit 1
fi

RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'
CYAN='\033[0;36m'; BOLD='\033[1m'; NC='\033[0m'

section() { echo -e "\n${CYAN}${BOLD}═══ $1 ═══${NC}"; }
ok()      { echo -e "  ${GREEN}[OK]${NC} $*";   CHECKS_OK+=("$*"); }
warn()    { echo -e "  ${YELLOW}[WARN]${NC} $*"; ISSUES+=("$*"); }
fail()    { echo -e "  ${RED}[FAIL]${NC} $*";    ISSUES+=("$*"); }
info()    { echo -e "  [INFO] $*"; }

# Parse arguments
DIAGNOSE_ONLY=false
JSON_OUTPUT=false
for arg in "$@"; do
    case "$arg" in
        --diagnose-only) DIAGNOSE_ONLY=true ;;
        --json)          JSON_OUTPUT=true ;;
    esac
done

# Tracking arrays
ISSUES=(); ACTIONS=(); FAILURES=(); CHECKS_OK=()

# State variables for JSON output
SVC_PIPEWIRE="unknown"; SVC_WIREPLUMBER="unknown"; SVC_BLUETOOTH="unknown"
SVC_SOUNDSYNC="unknown"; SVC_PIPEWIRE_PULSE="unknown"
BT_POWERED=false; BT_DISCOVERABLE=false; BT_PAIRABLE=false; BT_CLASS=""
PIPE_NULL_SINK=false; PIPE_EQ_SINK=false; PIPE_DEFAULT_SINK=""
BT_SPA_INSTALLED=false; WP_A2DP_CONFIG=false

# ── Detect service user ──────────────────────────────────────────────────────
detect_service_user() {
    # 1. Parse from systemd service file
    if [[ -f /etc/systemd/system/soundsync.service ]]; then
        local svc_user
        svc_user=$(grep -oP '^User=\K.*' /etc/systemd/system/soundsync.service 2>/dev/null || true)
        if [[ -n "$svc_user" ]] && id "$svc_user" &>/dev/null; then
            echo "$svc_user"
            return
        fi
    fi
    # 2. Fallback to SUDO_USER
    if [[ -n "${SUDO_USER:-}" ]] && [[ "$SUDO_USER" != "root" ]]; then
        echo "$SUDO_USER"
        return
    fi
    # 3. Fallback to first user running PipeWire
    local pw_user
    pw_user=$(ps -eo user,comm 2>/dev/null | awk '$2=="pipewire"{print $1; exit}')
    if [[ -n "$pw_user" ]] && [[ "$pw_user" != "root" ]]; then
        echo "$pw_user"
        return
    fi
    # 4. Last resort: first non-root user with UID >= 1000
    awk -F: '$3 >= 1000 && $1 != "nobody" {print $1; exit}' /etc/passwd
}

RUN_USER=$(detect_service_user)
RUN_UID=$(id -u "$RUN_USER" 2>/dev/null || echo "1000")
export XDG_RUNTIME_DIR="/run/user/${RUN_UID}"
export DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/${RUN_UID}/bus"

# Report directory
REPORT_DIR="/tmp/soundsync-doctor-$(date +%Y%m%d-%H%M%S)"
mkdir -p "$REPORT_DIR"

# ── Run command as the PipeWire session user ─────────────────────────────────
# Usage: run_as_user "command arg1 arg2"
# The entire command string must be ONE quoted argument.
run_as_user() {
    local cmd="$1"
    if [[ "$(whoami)" == "$RUN_USER" ]]; then
        bash -c "export XDG_RUNTIME_DIR=/run/user/${RUN_UID}; export DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/${RUN_UID}/bus; $cmd"
    else
        su - "$RUN_USER" -s /bin/bash -c "export XDG_RUNTIME_DIR=/run/user/${RUN_UID}; export DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/${RUN_UID}/bus; $cmd"
    fi
}

# ── Wait helpers ─────────────────────────────────────────────────────────────
wait_for_user_service() {
    local svc="$1" max="${2:-10}"
    for _ in $(seq 1 "$max"); do
        if run_as_user "systemctl --user is-active '$svc'" &>/dev/null; then
            return 0
        fi
        sleep 1
    done
    return 1
}

wait_for_system_service() {
    local svc="$1" max="${2:-10}"
    for _ in $(seq 1 "$max"); do
        if systemctl is-active "$svc" &>/dev/null; then
            return 0
        fi
        sleep 1
    done
    return 1
}

wait_for_pipewire_ready() {
    local max="${1:-15}"
    for _ in $(seq 1 "$max"); do
        if run_as_user "pactl info" &>/dev/null; then
            return 0
        fi
        sleep 1
    done
    return 1
}

# ── Cleanup on exit ──────────────────────────────────────────────────────────
cleanup() {
    # Nothing destructive to undo — report dir is intentionally kept
    :
}
trap cleanup EXIT

echo -e "${BOLD}SoundSync Doctor${NC} — $(date -Iseconds)"
echo "Service user: ${RUN_USER} (UID ${RUN_UID})"
echo "Report dir:   ${REPORT_DIR}"
if $DIAGNOSE_ONLY; then echo -e "${YELLOW}Mode: diagnose-only (no repairs)${NC}"; fi

# ═══════════════════════════════════════════════════════════════════════════════
# SECTION B: SYSTEM INSPECTION
# ═══════════════════════════════════════════════════════════════════════════════
section "System Inspection"

# ── B1: Process checks (detect duplicates) ───────────────────────────────────
check_process() {
    local name="$1"
    local count
    count=$(pgrep -c -x "$name" 2>/dev/null || echo "0")
    if [[ "$count" -eq 0 ]]; then
        fail "$name: not running"
    elif [[ "$count" -eq 1 ]]; then
        ok "$name: running (PID $(pgrep -x "$name" | head -1))"
    else
        fail "$name: $count instances running (expected 1)"
    fi
    echo "$count"
}

PW_COUNT=$(check_process "pipewire")
WP_COUNT=$(check_process "wireplumber")
check_process "bluetoothd" > /dev/null

# SoundSync may match multiple patterns — use -f
SS_COUNT=$(pgrep -c -f "/opt/soundsync/soundsync" 2>/dev/null || echo "0")
if [[ "$SS_COUNT" -eq 0 ]]; then
    fail "soundsync: not running"
elif [[ "$SS_COUNT" -eq 1 ]]; then
    ok "soundsync: running (PID $(pgrep -f '/opt/soundsync/soundsync' | head -1))"
else
    fail "soundsync: $SS_COUNT instances running (expected 1)"
fi

# ── B2: systemd service status ───────────────────────────────────────────────
for svc in pipewire wireplumber pipewire-pulse; do
    if run_as_user "systemctl --user is-active '$svc'" &>/dev/null; then
        ok "systemd --user $svc: active"
        eval "SVC_$(echo "$svc" | tr '-' '_' | tr '[:lower:]' '[:upper:]')=active"
    else
        fail "systemd --user $svc: inactive"
        eval "SVC_$(echo "$svc" | tr '-' '_' | tr '[:lower:]' '[:upper:]')=inactive"
    fi
done

if systemctl is-active bluetooth &>/dev/null; then
    ok "systemd bluetooth: active"; SVC_BLUETOOTH="active"
else
    fail "systemd bluetooth: inactive"; SVC_BLUETOOTH="inactive"
fi

if systemctl is-active soundsync &>/dev/null; then
    ok "systemd soundsync: active"; SVC_SOUNDSYNC="active"
else
    warn "systemd soundsync: inactive"; SVC_SOUNDSYNC="inactive"
fi

# ── B3: XDG_RUNTIME_DIR ─────────────────────────────────────────────────────
if [[ -d "/run/user/${RUN_UID}" ]]; then
    local_owner=$(stat -c '%U' "/run/user/${RUN_UID}" 2>/dev/null || echo "unknown")
    if [[ "$local_owner" == "$RUN_USER" ]]; then
        ok "XDG_RUNTIME_DIR exists, owned by $RUN_USER"
    else
        fail "XDG_RUNTIME_DIR owned by $local_owner (expected $RUN_USER)"
    fi
else
    fail "XDG_RUNTIME_DIR /run/user/${RUN_UID} does not exist"
fi

# ── B4: DBus session socket ──────────────────────────────────────────────────
if [[ -S "/run/user/${RUN_UID}/bus" ]]; then
    ok "DBus session socket exists"
else
    fail "DBus session socket /run/user/${RUN_UID}/bus missing"
fi

# ── B5: loginctl linger ─────────────────────────────────────────────────────
if loginctl show-user "$RUN_USER" 2>/dev/null | grep -q "Linger=yes"; then
    ok "loginctl linger enabled for $RUN_USER"
else
    warn "loginctl linger NOT enabled for $RUN_USER"
fi

# ── B6: Versions ─────────────────────────────────────────────────────────────
PW_VER=$(run_as_user "pw-cli --version 2>/dev/null | head -1" || echo "unknown")
WP_VER=$(wireplumber --version 2>/dev/null | grep -oP '\d+\.\d+\.\d+' | head -1 || echo "unknown")
info "PipeWire: $PW_VER"
info "WirePlumber: $WP_VER"

# ═══════════════════════════════════════════════════════════════════════════════
# SECTION C: BLUETOOTH VALIDATION
# ═══════════════════════════════════════════════════════════════════════════════
section "Bluetooth Validation"

# ── C1: Adapter ──────────────────────────────────────────────────────────────
BT_INFO=$(bluetoothctl show 2>/dev/null || echo "")
echo "$BT_INFO" > "$REPORT_DIR/bluetoothctl-show.txt"

if [[ -z "$BT_INFO" ]]; then
    fail "No Bluetooth adapter found"
else
    # Parse adapter properties
    BT_POWERED=$(echo "$BT_INFO" | grep -oP 'Powered: \K\w+' || echo "no")
    BT_DISCOVERABLE=$(echo "$BT_INFO" | grep -oP 'Discoverable: \K\w+' || echo "no")
    BT_PAIRABLE=$(echo "$BT_INFO" | grep -oP 'Pairable: \K\w+' || echo "no")
    BT_CLASS=$(echo "$BT_INFO" | grep -oP 'Class: \K\S+' || echo "")

    [[ "$BT_POWERED" == "yes" ]]       && ok "Adapter powered"          || fail "Adapter NOT powered"
    [[ "$BT_DISCOVERABLE" == "yes" ]]   && ok "Adapter discoverable"    || fail "Adapter NOT discoverable"
    [[ "$BT_PAIRABLE" == "yes" ]]       && ok "Adapter pairable"        || fail "Adapter NOT pairable"

    # C3: Class of Device check (0x240414 = Audio speaker)
    if [[ -n "$BT_CLASS" ]]; then
        info "Bluetooth Class: $BT_CLASS"
        if echo "$BT_CLASS" | grep -qiE "0x0*240414"; then
            ok "Class of Device is Audio/Speaker (0x240414)"
        elif echo "$BT_CLASS" | grep -qiE "0x0*040414"; then
            ok "Class of Device is Audio/Speaker ($BT_CLASS — acceptable)"
        else
            warn "Class of Device is $BT_CLASS (expected 0x240414 for speaker)"
        fi
    else
        warn "Could not determine Bluetooth Class of Device"
    fi
fi

# ── C4: /etc/bluetooth/main.conf ─────────────────────────────────────────────
if [[ -f /etc/bluetooth/main.conf ]]; then
    if grep -q "Class.*=.*0x240414" /etc/bluetooth/main.conf 2>/dev/null; then
        ok "main.conf has speaker Class"
    else
        warn "main.conf missing Class = 0x240414"
    fi
    if grep -q "DiscoverableTimeout.*=.*0" /etc/bluetooth/main.conf 2>/dev/null; then
        ok "main.conf: DiscoverableTimeout = 0 (always discoverable)"
    else
        warn "main.conf: DiscoverableTimeout not set to 0"
    fi
else
    fail "/etc/bluetooth/main.conf does not exist"
fi

# ── C5: libspa-0.2-bluetooth ─────────────────────────────────────────────────
if find /usr/lib -path "*/spa-0.2/bluez5" -type d 2>/dev/null | grep -q .; then
    ok "libspa-0.2-bluetooth SPA plugin installed"
    BT_SPA_INSTALLED=true
else
    fail "libspa-0.2-bluetooth NOT installed — Bluetooth audio cannot work"
    BT_SPA_INSTALLED=false
fi

# ── C6: WirePlumber A2DP config (MUST match WP version) ─────────────────────
WP_A2DP_CONFIG=false
WP_A2DP_CONFIG_PATH=""
WP_A2DP_WRONG_FORMAT=false

# Determine if this is WP 0.4.x or 0.5+
WP_IS_05=false
if [[ -n "$WP_VER" ]] && [[ "$WP_VER" != "unknown" ]]; then
    WP_MAJOR_MINOR=$(echo "$WP_VER" | grep -oP '^\d+\.\d+' || echo "0.4")
    if dpkg --compare-versions "$WP_MAJOR_MINOR" ge "0.5" 2>/dev/null; then
        WP_IS_05=true
    fi
fi
info "WirePlumber config format: $($WP_IS_05 && echo '0.5+ (.conf)' || echo '0.4.x (Lua)')"

if $WP_IS_05; then
    # WP 0.5+: check conf.d
    for f in /etc/wireplumber/wireplumber.conf.d/51-soundsync*; do
        if [[ -f "$f" ]] && grep -q "a2dp_sink" "$f" 2>/dev/null; then
            ok "WirePlumber A2DP config found: $f (correct format for WP 0.5+)"
            WP_A2DP_CONFIG=true; WP_A2DP_CONFIG_PATH="$f"
            break
        fi
    done
else
    # WP 0.4.x: check Lua dirs
    for dir in /etc/wireplumber/bluetooth.lua.d \
               "$(eval echo "~${RUN_USER}")/.config/wireplumber/bluetooth.lua.d"; do
        for f in "$dir"/51-soundsync*; do
            if [[ -f "$f" ]] && grep -q "a2dp_sink" "$f" 2>/dev/null; then
                # Check if config uses BROKEN table-replacement syntax
                if grep -q 'bluez_monitor\.properties\s*=' "$f" 2>/dev/null; then
                    fail "Found $f but it uses table-replacement syntax (wipes defaults!)"
                    info "  Must use: bluez_monitor.properties[\"key\"] = value"
                    info "  Not:      bluez_monitor.properties = { ... }"
                    WP_A2DP_WRONG_FORMAT=true
                else
                    ok "WirePlumber A2DP config found: $f (correct format for WP 0.4.x)"
                    WP_A2DP_CONFIG=true; WP_A2DP_CONFIG_PATH="$f"
                fi
                break 2
            fi
        done
    done

    # Check if a .conf file exists but is WRONG format for 0.4.x
    for f in /etc/wireplumber/wireplumber.conf.d/51-soundsync*; do
        if [[ -f "$f" ]] && grep -q "a2dp_sink" "$f" 2>/dev/null; then
            fail "Found $f but WP $WP_VER is 0.4.x — this .conf file is IGNORED!"
            info "  WP 0.4.x needs Lua config in bluetooth.lua.d/, not .conf in wireplumber.conf.d/"
            WP_A2DP_WRONG_FORMAT=true
            break
        fi
    done
fi

if ! $WP_A2DP_CONFIG; then
    fail "WirePlumber A2DP sink config NOT found (or wrong format for WP $WP_VER)"
fi

# ── C7: Detect conflicting audio servers ─────────────────────────────────────
CONFLICTING_AUDIO=false

# Check for real PulseAudio (not PipeWire's pipewire-pulse)
if pgrep -x "pulseaudio" &>/dev/null; then
    fail "PulseAudio daemon running — conflicts with PipeWire Bluetooth!"
    info "  PulseAudio and PipeWire cannot both manage Bluetooth audio"
    CONFLICTING_AUDIO=true
fi

# Check for bluez-alsa
if pgrep -f "bluealsa" &>/dev/null; then
    fail "bluez-alsa (bluealsa) running — steals BT transports from WirePlumber!"
    CONFLICTING_AUDIO=true
fi
if systemctl is-active --quiet bluealsa 2>/dev/null; then
    fail "bluealsa.service is active — must be stopped for BT audio to work"
    CONFLICTING_AUDIO=true
fi

if ! $CONFLICTING_AUDIO; then
    ok "No conflicting audio servers detected"
fi

# ── C8: D-Bus access to BlueZ (CRITICAL for BT audio) ───────────────────────
DBUS_BLUEZ_OK=false

# Test if the service user can access org.bluez on the system bus
DBUS_TEST=$(run_as_user "dbus-send --system --print-reply --dest=org.bluez / org.freedesktop.DBus.ObjectManager.GetManagedObjects 2>&1" || echo "FAILED")
if echo "$DBUS_TEST" | grep -q "array"; then
    ok "D-Bus access to org.bluez works"
    DBUS_BLUEZ_OK=true
else
    fail "Cannot access org.bluez over system D-Bus — BlueZ5 SPA plugin will fail!"
    DBUS_ERROR=$(echo "$DBUS_TEST" | head -3)
    info "  Error: $DBUS_ERROR"

    # Check if user is in bluetooth group
    if groups "$RUN_USER" 2>/dev/null | grep -q "bluetooth"; then
        info "  User $RUN_USER IS in bluetooth group"
    else
        fail "  User $RUN_USER is NOT in bluetooth group"
        info "  Fix: sudo usermod -aG bluetooth $RUN_USER"
    fi

    # Check D-Bus policy file
    if [[ -f /etc/dbus-1/system.d/bluetooth.conf ]] || [[ -f /usr/share/dbus-1/system.d/bluetooth.conf ]]; then
        info "  BlueZ D-Bus policy file exists"
        # Check if it allows access for bluetooth group or at_console users
        local policy_file
        policy_file=$(ls /etc/dbus-1/system.d/bluetooth.conf /usr/share/dbus-1/system.d/bluetooth.conf 2>/dev/null | head -1)
        if grep -q 'group="bluetooth"' "$policy_file" 2>/dev/null; then
            info "  Policy allows bluetooth group access"
        elif grep -q 'at_console="true"' "$policy_file" 2>/dev/null; then
            info "  Policy allows console users"
        else
            warn "  D-Bus policy may not allow user access to BlueZ"
        fi
    else
        fail "  No BlueZ D-Bus policy file found!"
    fi
fi

# ── C9: SPA BlueZ5 module load test ─────────────────────────────────────────
SPA_BLUEZ_LOADS=false
SPA_TEST=$(run_as_user "spa-inspect /usr/lib/x86_64-linux-gnu/spa-0.2/bluez5/libspa-bluez5.so 2>&1" | head -5 || echo "FAILED")
if echo "$SPA_TEST" | grep -q "factory name.*api.bluez5"; then
    ok "SPA bluez5 plugin loads and exports factories"
    SPA_BLUEZ_LOADS=true
else
    # Try alternate path
    SPA_PATH=$(find /usr/lib -name "libspa-bluez5.so" -type f 2>/dev/null | head -1)
    if [[ -n "$SPA_PATH" ]]; then
        SPA_TEST=$(run_as_user "spa-inspect $SPA_PATH 2>&1" | head -5 || echo "FAILED")
        if echo "$SPA_TEST" | grep -q "factory name.*api.bluez5"; then
            ok "SPA bluez5 plugin loads from $SPA_PATH"
            SPA_BLUEZ_LOADS=true
        fi
    fi
    if ! $SPA_BLUEZ_LOADS; then
        warn "SPA bluez5 plugin may have issues loading"
        info "  $SPA_TEST"
    fi
fi

# ═══════════════════════════════════════════════════════════════════════════════
# SECTION D: AUDIO PIPELINE VALIDATION
# ═══════════════════════════════════════════════════════════════════════════════
section "Audio Pipeline"

# Save full state for debugging
run_as_user "pw-cli list-objects" > "$REPORT_DIR/pw-objects.txt" 2>/dev/null || true
run_as_user "pw-dump" > "$REPORT_DIR/pw-dump.json" 2>/dev/null || true
run_as_user "pactl list short sinks" > "$REPORT_DIR/pa-sinks.txt" 2>/dev/null || true
run_as_user "pactl list short sources" > "$REPORT_DIR/pa-sources.txt" 2>/dev/null || true
run_as_user "pactl list short modules" > "$REPORT_DIR/pa-modules.txt" 2>/dev/null || true
run_as_user "pw-link -l" > "$REPORT_DIR/pw-links.txt" 2>/dev/null || true

# ── D1: Null sink ────────────────────────────────────────────────────────────
if grep -q "soundsync-capture" "$REPORT_DIR/pa-sinks.txt" 2>/dev/null; then
    ok "soundsync-capture null sink exists"
    PIPE_NULL_SINK=true
else
    info "soundsync-capture null sink not present (created when SoundSync starts)"
    PIPE_NULL_SINK=false
fi

# ── D2: EQ filter-chain ─────────────────────────────────────────────────────
if grep -q "effect_input.soundsync-eq" "$REPORT_DIR/pa-sinks.txt" 2>/dev/null; then
    ok "EQ filter-chain sink exists"
    PIPE_EQ_SINK=true
else
    info "EQ filter-chain not active (starts when EQ is enabled)"
    PIPE_EQ_SINK=false
fi

# ── D3: Default sink ────────────────────────────────────────────────────────
PIPE_DEFAULT_SINK=$(run_as_user "pactl get-default-sink" 2>/dev/null || echo "unknown")
info "Default sink: $PIPE_DEFAULT_SINK"
if echo "$PIPE_DEFAULT_SINK" | grep -q "soundsync\|effect_input"; then
    ok "Default sink routed to SoundSync"
elif [[ "$SVC_SOUNDSYNC" == "inactive" ]]; then
    info "SoundSync not running — default sink not expected to be set"
else
    warn "Default sink ($PIPE_DEFAULT_SINK) not routed to SoundSync"
fi

# ── D4: Duplicate modules ───────────────────────────────────────────────────
DUPE_COUNT=$(grep -c "soundsync" "$REPORT_DIR/pa-modules.txt" 2>/dev/null | tr -dc '0-9' || echo "0")
DUPE_COUNT=${DUPE_COUNT:-0}
if [[ "$DUPE_COUNT" -gt 2 ]]; then
    fail "Found $DUPE_COUNT SoundSync modules (expected <=2) — duplicates exist"
elif [[ "$DUPE_COUNT" -gt 0 ]]; then
    ok "SoundSync modules: $DUPE_COUNT (within normal range)"
fi

# ── D5: Bluetooth audio nodes ───────────────────────────────────────────────
BT_NODE_COUNT=$(grep -c "bluez_input\|bluez_source" "$REPORT_DIR/pw-objects.txt" 2>/dev/null | tr -dc '0-9' || echo "0")
BT_NODE_COUNT=${BT_NODE_COUNT:-0}
if [[ "$BT_NODE_COUNT" -gt 0 ]]; then
    ok "Bluetooth audio nodes in PipeWire: $BT_NODE_COUNT"
else
    info "No Bluetooth audio nodes (no device currently streaming)"
fi

# ── D6: PipeWire links ──────────────────────────────────────────────────────
LINK_COUNT=$(wc -l < "$REPORT_DIR/pw-links.txt" 2>/dev/null || echo "0")
info "PipeWire links: ~$LINK_COUNT entries"

# ── D7: Capture process ─────────────────────────────────────────────────────
if pgrep -f "parec.*soundsync\|pw-cat.*soundsync" &>/dev/null; then
    ok "Audio capture process running"
elif [[ "$SVC_SOUNDSYNC" == "active" ]]; then
    warn "SoundSync is active but no capture process found"
else
    info "No capture process (SoundSync not running)"
fi

# ── D8: BlueZ5 devices in PipeWire (CRITICAL for BT audio) ─────────────────
BLUEZ5_DEVICE_COUNT=$(grep -c "device.api.*=.*bluez5\|api.bluez5" "$REPORT_DIR/pw-objects.txt" 2>/dev/null | tr -dc '0-9' || echo "0")
BLUEZ5_DEVICE_COUNT=${BLUEZ5_DEVICE_COUNT:-0}
if [[ "$BLUEZ5_DEVICE_COUNT" -gt 0 ]]; then
    ok "WirePlumber BlueZ5 monitor active ($BLUEZ5_DEVICE_COUNT device(s))"
else
    fail "No BlueZ5 devices in PipeWire — WirePlumber cannot see Bluetooth hardware"
    info "  Root cause: libspa-0.2-bluetooth missing OR WirePlumber A2DP config not loaded"
fi

# ── D9: Default sink routing check ──────────────────────────────────────────
if [[ "$SVC_SOUNDSYNC" == "active" ]] && $PIPE_NULL_SINK; then
    if echo "$PIPE_DEFAULT_SINK" | grep -q "soundsync-capture\|effect_input.soundsync-eq"; then
        ok "Default sink correctly routes to SoundSync pipeline"
    else
        fail "Default sink is '$PIPE_DEFAULT_SINK' — BT audio will NOT reach SoundSync"
        info "  Expected: soundsync-capture or effect_input.soundsync-eq"
    fi
fi

# ═══════════════════════════════════════════════════════════════════════════════
# SECTION E: SELF-HEALING REPAIR
# ═══════════════════════════════════════════════════════════════════════════════
repair_system() {
    section "Self-Healing Repair"

    # ── E1: Stop services safely ─────────────────────────────────────────────
    info "Step 1: Stopping services..."
    if systemctl is-active soundsync &>/dev/null; then
        systemctl stop soundsync 2>/dev/null || true
        ACTIONS+=("Stopped soundsync service")
    fi
    run_as_user "systemctl --user stop wireplumber" 2>/dev/null || true
    run_as_user "systemctl --user stop pipewire-pulse" 2>/dev/null || true
    run_as_user "systemctl --user stop pipewire" 2>/dev/null || true
    sleep 1

    # ── E2: Kill orphaned processes ──────────────────────────────────────────
    info "Step 2: Cleaning orphaned processes..."
    for pattern in "pipewire-filter-chain" "pw-loopback" "parec.*soundsync" "pw-cat.*soundsync"; do
        if pgrep -u "$RUN_UID" -f "$pattern" &>/dev/null; then
            pkill -u "$RUN_UID" -f "$pattern" 2>/dev/null || true
            ACTIONS+=("Killed orphaned: $pattern")
        fi
    done

    # Kill duplicate pipewire/wireplumber instances
    for proc in pipewire wireplumber; do
        local count
        count=$(pgrep -u "$RUN_UID" -c -x "$proc" 2>/dev/null | tr -dc '0-9' || echo "0")
        count=${count:-0}
        if [[ "$count" -gt 1 ]]; then
            # Kill all — they'll be restarted cleanly
            pkill -u "$RUN_UID" -x "$proc" 2>/dev/null || true
            ACTIONS+=("Killed $count duplicate $proc instances")
        fi
    done
    sleep 1

    # ── E3: Clean PipeWire runtime state ─────────────────────────────────────
    info "Step 3: Cleaning stale runtime files..."
    if ! pgrep -u "$RUN_UID" -x pipewire &>/dev/null; then
        # Only clean lock files when PipeWire is stopped
        for f in /run/user/${RUN_UID}/pipewire-0.lock; do
            if [[ -f "$f" ]]; then
                rm -f "$f"
                ACTIONS+=("Removed stale lock: $f")
            fi
        done
    fi

    # ── E4: Fix environment ──────────────────────────────────────────────────
    info "Step 4: Fixing environment..."
    if [[ ! -d "/run/user/${RUN_UID}" ]]; then
        mkdir -p "/run/user/${RUN_UID}"
        chown "${RUN_USER}:${RUN_USER}" "/run/user/${RUN_UID}"
        chmod 700 "/run/user/${RUN_UID}"
        ACTIONS+=("Created XDG_RUNTIME_DIR")
    fi

    if ! loginctl show-user "$RUN_USER" 2>/dev/null | grep -q "Linger=yes"; then
        loginctl enable-linger "$RUN_USER" 2>/dev/null || true
        ACTIONS+=("Enabled loginctl linger for $RUN_USER")
    fi

    run_as_user "systemctl --user daemon-reload" 2>/dev/null || true
    run_as_user "systemctl --user enable pipewire.service pipewire-pulse.service wireplumber.service" 2>/dev/null || true

    # ── E5: Install missing SPA plugin ─────────────────────────────────────────
    info "Step 5a: Checking libspa-0.2-bluetooth..."
    if ! $BT_SPA_INSTALLED; then
        if command -v apt-get &>/dev/null; then
            info "Installing libspa-0.2-bluetooth (MANDATORY for BT audio)..."
            apt-get install -y -qq libspa-0.2-bluetooth 2>/dev/null || true
            # Verify installation
            if find /usr/lib -path "*/spa-0.2/bluez5" -type d 2>/dev/null | grep -q .; then
                ok "libspa-0.2-bluetooth installed successfully"
                BT_SPA_INSTALLED=true
                ACTIONS+=("Installed libspa-0.2-bluetooth")
            else
                fail "Failed to install libspa-0.2-bluetooth"
                FAILURES+=("libspa-0.2-bluetooth installation failed")
            fi
        else
            fail "Cannot auto-install libspa-0.2-bluetooth (apt-get not found)"
            FAILURES+=("libspa-0.2-bluetooth missing, no package manager")
        fi
    fi

    # ── E5b: Fix Bluetooth config ────────────────────────────────────────────
    info "Step 5b: Checking Bluetooth config..."
    local bt_needs_fix=false
    if [[ ! -f /etc/bluetooth/main.conf ]] || \
       ! grep -q "Class.*=.*0x240414" /etc/bluetooth/main.conf 2>/dev/null || \
       ! grep -q "DiscoverableTimeout.*=.*0" /etc/bluetooth/main.conf 2>/dev/null || \
       ! grep -q "Name.*=.*SoundSync" /etc/bluetooth/main.conf 2>/dev/null; then
        bt_needs_fix=true
    fi

    if $bt_needs_fix; then
        info "Writing corrected /etc/bluetooth/main.conf..."
        if [[ -f /etc/bluetooth/main.conf ]]; then
            cp /etc/bluetooth/main.conf "/etc/bluetooth/main.conf.bak.$(date +%s)" 2>/dev/null || true
        fi
        cat > /etc/bluetooth/main.conf << 'BTCONF'
[General]
Class = 0x240414
Name = SoundSync
DiscoverableTimeout = 0
PairableTimeout = 0
Discoverable = true
Pairable = true

[Policy]
AutoEnable = true
BTCONF
        ACTIONS+=("Wrote /etc/bluetooth/main.conf with speaker Class 0x240414")
    fi

    # ── E5c: Stop conflicting audio servers ─────────────────────────────────
    if $CONFLICTING_AUDIO; then
        info "Step 5c: Stopping conflicting audio servers..."
        if pgrep -x "pulseaudio" &>/dev/null; then
            killall pulseaudio 2>/dev/null || true
            # Prevent PulseAudio from auto-respawning
            if [[ -f /etc/pulse/client.conf ]]; then
                if ! grep -q "autospawn = no" /etc/pulse/client.conf 2>/dev/null; then
                    echo "autospawn = no" >> /etc/pulse/client.conf
                fi
            fi
            ACTIONS+=("Killed PulseAudio daemon")
        fi
        if pgrep -f "bluealsa" &>/dev/null; then
            killall bluealsa 2>/dev/null || true
            ACTIONS+=("Killed bluez-alsa")
        fi
        if systemctl is-active --quiet bluealsa 2>/dev/null; then
            systemctl stop bluealsa 2>/dev/null || true
            systemctl disable bluealsa 2>/dev/null || true
            ACTIONS+=("Stopped and disabled bluealsa.service")
        fi
    fi

    # ── E5d: Fix D-Bus access to BlueZ ──────────────────────────────────────
    if ! $DBUS_BLUEZ_OK; then
        info "Step 5d: Fixing D-Bus access to BlueZ..."

        # Add user to bluetooth group
        if ! groups "$RUN_USER" 2>/dev/null | grep -q "bluetooth"; then
            usermod -aG bluetooth "$RUN_USER" 2>/dev/null || true
            ok "Added $RUN_USER to bluetooth group"
            ACTIONS+=("Added $RUN_USER to bluetooth group")
        fi

        # Ensure D-Bus policy allows bluetooth group
        local bt_dbus_policy="/etc/dbus-1/system.d/bluetooth.conf"
        if [[ ! -f "$bt_dbus_policy" ]]; then
            bt_dbus_policy="/usr/share/dbus-1/system.d/bluetooth.conf"
        fi

        if [[ -f "$bt_dbus_policy" ]]; then
            # Check if policy already allows bluetooth group
            if ! grep -q 'group="bluetooth"' "$bt_dbus_policy" 2>/dev/null; then
                # Create a supplementary policy that grants bluetooth group access
                cat > /etc/dbus-1/system.d/soundsync-bluetooth.conf << 'DBUSCONF'
<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-BUS Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <!-- Allow bluetooth group to access BlueZ (needed for PipeWire/WirePlumber) -->
  <policy group="bluetooth">
    <allow send_destination="org.bluez"/>
    <allow send_interface="org.bluez.Agent1"/>
    <allow send_interface="org.bluez.MediaEndpoint1"/>
    <allow send_interface="org.bluez.MediaTransport1"/>
    <allow send_interface="org.bluez.Profile1"/>
    <allow send_interface="org.freedesktop.DBus.ObjectManager"/>
    <allow send_interface="org.freedesktop.DBus.Properties"/>
  </policy>
</busconfig>
DBUSCONF
                ok "Created D-Bus policy: /etc/dbus-1/system.d/soundsync-bluetooth.conf"
                ACTIONS+=("Created D-Bus policy for bluetooth group")

                # Reload D-Bus to pick up new policy
                systemctl reload dbus 2>/dev/null || killall -HUP dbus-daemon 2>/dev/null || true
                sleep 1
                ACTIONS+=("Reloaded D-Bus daemon")
            else
                info "D-Bus policy already allows bluetooth group"
            fi
        fi

        # Verify D-Bus access now works
        local dbus_recheck
        dbus_recheck=$(run_as_user "dbus-send --system --print-reply --dest=org.bluez / org.freedesktop.DBus.ObjectManager.GetManagedObjects 2>&1" || echo "FAILED")
        if echo "$dbus_recheck" | grep -q "array"; then
            ok "D-Bus access to org.bluez now works"
            DBUS_BLUEZ_OK=true
        else
            warn "D-Bus access still failing — user may need to log out and back in for group change"
            info "  Workaround: the service restart later will use the updated group membership"
            FAILURES+=("D-Bus access to org.bluez still failing after group fix")
        fi
    fi

    # ── E6: Fix WirePlumber A2DP config ──────────────────────────────────────
    info "Step 6: Checking WirePlumber A2DP config..."

    # If wrong format exists, remove ALL old soundsync WP configs first
    if $WP_A2DP_WRONG_FORMAT; then
        info "Removing wrong-format WP config files..."
        for searchdir in \
            /etc/wireplumber/wireplumber.conf.d \
            /etc/wireplumber/bluetooth.lua.d \
            "$(eval echo "~${RUN_USER}")/.config/wireplumber/bluetooth.lua.d"; do
            for f in "$searchdir"/51-soundsync*; do
                if [[ -f "$f" ]]; then
                    rm -f "$f"
                    ACTIONS+=("Removed wrong-format WP config: $f")
                fi
            done
        done
    fi

    if ! $WP_A2DP_CONFIG || $WP_A2DP_WRONG_FORMAT; then
        if $WP_IS_05; then
            mkdir -p /etc/wireplumber/wireplumber.conf.d
            cat > /etc/wireplumber/wireplumber.conf.d/51-soundsync.conf << 'WPCONF'
# SoundSync: Enable A2DP sink role
monitor.bluez.properties = {
    bluez5.roles = [ a2dp_sink ]
    bluez5.codecs = [ sbc aac ldac aptx aptx_hd ]
    bluez5.enable-sbc-xq = true
    bluez5.enable-msbc = false
    bluez5.enable-hw-volume = true
    bluez5.a2dp.opus.pro.channels = 0
}
WPCONF
            ok "Wrote WirePlumber 0.5+ A2DP config"
            ACTIONS+=("Wrote WirePlumber 0.5+ A2DP config to wireplumber.conf.d/")
        else
            # WP 0.4.x: Write Lua config (both system and user locations)
            mkdir -p /etc/wireplumber/bluetooth.lua.d
            cat > /etc/wireplumber/bluetooth.lua.d/51-soundsync.lua << 'WPLUA'
-- SoundSync: Enable A2DP sink role for Bluetooth audio reception
-- IMPORTANT: Modify individual properties — do NOT replace the entire
-- bluez_monitor.properties table, as that wipes defaults like with-logind
-- which are critical for the BlueZ monitor to activate.
bluez_monitor.properties["bluez5.roles"] = "[ a2dp_sink ]"
bluez_monitor.properties["bluez5.codecs"] = "[ sbc aac ldac aptx aptx_hd ]"
bluez_monitor.properties["bluez5.enable-sbc-xq"] = true
bluez_monitor.properties["bluez5.enable-msbc"] = false
bluez_monitor.properties["bluez5.enable-hw-volume"] = true
WPLUA
            ok "Wrote WirePlumber 0.4.x Lua A2DP config to /etc/wireplumber/bluetooth.lua.d/"
            ACTIONS+=("Wrote WirePlumber 0.4.x Lua A2DP config")

            # Also write to user config dir for extra reliability
            local user_wp_dir
            user_wp_dir="$(eval echo "~${RUN_USER}")/.config/wireplumber/bluetooth.lua.d"
            mkdir -p "$user_wp_dir"
            cp /etc/wireplumber/bluetooth.lua.d/51-soundsync.lua "$user_wp_dir/51-soundsync-a2dp.lua"
            chown -R "${RUN_USER}:${RUN_USER}" "$(eval echo "~${RUN_USER}")/.config/wireplumber" 2>/dev/null || true
            ACTIONS+=("Wrote WP 0.4.x user config to $user_wp_dir/")
        fi
        WP_A2DP_CONFIG=true
    fi

    # ── E7: Restart services in order with verification ──────────────────────
    info "Step 7: Restarting services..."
    info "  Order: Bluetooth FIRST → PipeWire → PipeWire-Pulse → WirePlumber"
    info "  (WirePlumber must start AFTER Bluetooth is stable to avoid proxy destroyed)"

    # Bluetooth FIRST — must be stable before WirePlumber starts
    systemctl restart bluetooth 2>/dev/null || true
    sleep 2
    if wait_for_system_service "bluetooth" 10; then
        ok "bluetooth restarted"; ACTIONS+=("Restarted bluetooth")
    else
        fail "bluetooth failed to restart"; FAILURES+=("bluetooth failed to restart")
    fi

    # Verify Bluetooth adapter is ready
    sleep 1
    local bt_check
    bt_check=$(bluetoothctl show 2>/dev/null || echo "")

    # Wait for BlueZ to fully register on D-Bus
    for _btw in 1 2 3 4 5; do
        if run_as_user "dbus-send --system --print-reply --dest=org.bluez / org.freedesktop.DBus.ObjectManager.GetManagedObjects 2>/dev/null" | grep -q "array" 2>/dev/null; then
            ok "BlueZ D-Bus ready"
            break
        fi
        sleep 1
    done

    # PipeWire
    run_as_user "systemctl --user start pipewire.service" 2>/dev/null || true
    if wait_for_user_service "pipewire" 10; then
        ok "pipewire started"; ACTIONS+=("Started pipewire")
    else
        fail "pipewire failed to start"; FAILURES+=("pipewire failed to start")
    fi

    # PipeWire-Pulse
    run_as_user "systemctl --user start pipewire-pulse.service" 2>/dev/null || true
    if wait_for_user_service "pipewire-pulse" 10; then
        ok "pipewire-pulse started"; ACTIONS+=("Started pipewire-pulse")
    else
        fail "pipewire-pulse failed to start"; FAILURES+=("pipewire-pulse failed to start")
    fi

    # Verify PipeWire accepts commands BEFORE starting WirePlumber
    if wait_for_pipewire_ready 15; then
        ok "PipeWire ready (pactl responding)"
    else
        fail "PipeWire not responding to pactl after restart"
        FAILURES+=("PipeWire not responding to pactl")
    fi

    # WirePlumber LAST — needs both PipeWire and Bluetooth stable
    run_as_user "systemctl --user start wireplumber.service" 2>/dev/null || true
    if wait_for_user_service "wireplumber" 10; then
        ok "wireplumber started"; ACTIONS+=("Started wireplumber")
    else
        fail "wireplumber failed to start"; FAILURES+=("wireplumber failed to start")
    fi

    # Give WirePlumber time to enumerate BlueZ devices
    sleep 3
    if echo "$bt_check" | grep -q "Powered: yes"; then
        ok "Bluetooth adapter powered after restart"
    else
        warn "Bluetooth adapter not powered — attempting to power on..."
        bluetoothctl power on &>/dev/null || true
        sleep 1
    fi
    if echo "$bt_check" | grep -q "Discoverable: yes"; then
        ok "Bluetooth adapter discoverable"
    else
        bluetoothctl discoverable on &>/dev/null || true
        ACTIONS+=("Set Bluetooth discoverable")
    fi

    # ── E7b: Force Bluetooth adapter state ─────────────────────────────────────
    info "Step 7b: Enforcing Bluetooth adapter state..."
    sleep 2

    # Force adapter power, discoverable, pairable via bluetoothctl
    bluetoothctl power on &>/dev/null || true
    sleep 1
    bluetoothctl discoverable on &>/dev/null || true
    bluetoothctl pairable on &>/dev/null || true
    sleep 1

    # Force Class of Device via multiple methods
    local post_class
    post_class=$(bluetoothctl show 2>/dev/null | grep -oP 'Class: \K\S+' || echo "unknown")
    info "Current BT Class after restart: $post_class"

    if ! echo "$post_class" | grep -qiE "0x0*240414|0x0*040414"; then
        warn "Class $post_class != 0x240414 — forcing via all available methods..."

        # Method 1: hciconfig (deprecated but still works on many systems)
        # Note: hciconfig may only set lower bits, resulting in 0x040414 instead of 0x240414
        if command -v hciconfig &>/dev/null; then
            hciconfig hci0 class 0x240414 2>/dev/null && \
                ACTIONS+=("Set BT class via hciconfig hci0 class 0x240414") || true
        fi

        # Method 2: btmgmt (modern replacement for hciconfig)
        # btmgmt class <major> <minor>: major=4 (Audio/Video), minor=20 (0x14=Loudspeaker)
        if command -v btmgmt &>/dev/null; then
            btmgmt class 4 20 2>/dev/null && \
                ACTIONS+=("Set BT class via btmgmt class 4 20") || true
        fi

        # Method 3: Direct HCI via Python (sets all 3 bytes of Class of Device)
        if command -v python3 &>/dev/null; then
            python3 -c "
import socket, struct
try:
    s = socket.socket(31, socket.SOCK_RAW, 1)  # AF_BLUETOOTH, HCI
    # HCI_Write_Class_of_Device: OGF=0x03, OCF=0x0024 -> opcode 0x0c24
    # Class 0x240414 -> little-endian bytes: [0x14, 0x04, 0x24]
    cmd = struct.pack('<HB3s', 0x0c24, 3, bytes([0x14, 0x04, 0x24]))
    s.send(cmd)
    s.close()
    print('HCI class set to 0x240414')
except Exception as e:
    print(f'HCI class set failed: {e}')
" 2>&1 | while IFS= read -r l; do info "  $l"; done
            ACTIONS+=("Set BT class via raw HCI command")
        fi

        # Re-check
        sleep 1
        post_class=$(bluetoothctl show 2>/dev/null | grep -oP 'Class: \K\S+' || echo "unknown")
        if echo "$post_class" | grep -qiE "0x0*240414"; then
            ok "Bluetooth Class is now 0x240414 (Audio/Speaker with rendering)"
        elif echo "$post_class" | grep -qiE "0x0*040414"; then
            ok "Bluetooth Class is 0x040414 (Audio/Speaker — rendering bit may differ)"
            info "  Phone should still see this as an audio device"
        else
            warn "Bluetooth Class is still $post_class"
            FAILURES+=("BT Class of Device stuck at $post_class (expected 0x240414)")
        fi
    else
        ok "Bluetooth Class is correct ($post_class)"
    fi

    # Force adapter name
    local post_name
    post_name=$(bluetoothctl show 2>/dev/null | grep -oP 'Name: \K.*' || echo "unknown")
    if [[ "$post_name" != "SoundSync" ]]; then
        info "BT name is '$post_name', setting to SoundSync..."
        bluetoothctl system-alias SoundSync &>/dev/null || true
        # Also try via btmgmt
        if command -v btmgmt &>/dev/null; then
            btmgmt name SoundSync 2>/dev/null || true
        fi
        ACTIONS+=("Set Bluetooth name to SoundSync")
    else
        ok "Bluetooth adapter name is SoundSync"
    fi

    # SoundSync
    if [[ -f /etc/systemd/system/soundsync.service ]]; then
        systemctl start soundsync 2>/dev/null || true
        if wait_for_system_service "soundsync" 15; then
            ok "soundsync started"; ACTIONS+=("Started soundsync")
        else
            fail "soundsync failed to start"; FAILURES+=("soundsync failed to start")
        fi
    else
        warn "soundsync.service not installed — skipping"
    fi

    # ── E8: Set default sink and clean duplicates ────────────────────────────
    info "Step 8: Setting default sink and cleaning duplicates..."
    sleep 3  # Give SoundSync time to create its null sink and start parec

    # Set soundsync-capture as default sink so BT audio routes there
    if run_as_user "pactl list short sinks 2>/dev/null" | grep -q "soundsync-capture"; then
        # If EQ is active, use its input as default; otherwise use capture directly
        if run_as_user "pactl list short sinks 2>/dev/null" | grep -q "effect_input.soundsync-eq"; then
            run_as_user "pactl set-default-sink effect_input.soundsync-eq" 2>/dev/null || true
            ok "Default sink set to effect_input.soundsync-eq (EQ enabled)"
            ACTIONS+=("Set default sink to effect_input.soundsync-eq")
        else
            run_as_user "pactl set-default-sink soundsync-capture" 2>/dev/null || true
            ok "Default sink set to soundsync-capture"
            ACTIONS+=("Set default sink to soundsync-capture")
        fi
    else
        warn "soundsync-capture sink not found — SoundSync may not have started properly"
    fi

    # Clean duplicate modules
    local mod_count
    mod_count=$(run_as_user "pactl list short modules 2>/dev/null | grep -c 'soundsync' | tr -dc '0-9'" || echo "0")
    mod_count=${mod_count:-0}
    if [[ "$mod_count" -gt 2 ]]; then
        warn "Found $mod_count SoundSync modules — cleaning duplicates"
        run_as_user "pactl list short modules" 2>/dev/null | \
            grep -E "module-(null-sink|loopback)" | \
            while read -r id name args; do
                case "$args" in
                    *soundsync*)
                        run_as_user "pactl unload-module $id" 2>/dev/null || true
                        ACTIONS+=("Unloaded duplicate module $id")
                        ;;
                esac
            done
    fi

    # ── E9: Verify WirePlumber BlueZ monitor is active ───────────────────────
    info "Step 9: Verifying WirePlumber BlueZ5 monitor..."

    # WirePlumber may need a few seconds after start to enumerate BT devices
    local bluez_device=0
    for attempt in 1 2 3 4 5; do
        bluez_device=$(run_as_user "pw-cli list-objects 2>/dev/null" | grep -c "device.api.*=.*bluez5\|api.bluez5" | tr -dc '0-9' || echo "0")
        bluez_device=${bluez_device:-0}
        if [[ "$bluez_device" -gt 0 ]]; then
            break
        fi
        sleep 2
    done

    if [[ "$bluez_device" -gt 0 ]]; then
        ok "WirePlumber BlueZ5 monitor active ($bluez_device device(s) in PipeWire)"
    else
        warn "No BlueZ5 devices in PipeWire graph after 10s"

        # Diagnose: check if SPA plugin files exist
        local spa_bluez_path
        spa_bluez_path=$(find /usr/lib -path "*/spa-0.2/bluez5" -type d 2>/dev/null | head -1)
        if [[ -n "$spa_bluez_path" ]]; then
            info "SPA bluez5 plugin dir exists: $spa_bluez_path"
            info "  Contents: $(ls "$spa_bluez_path" 2>/dev/null | tr '\n' ' ')"
        else
            fail "SPA bluez5 plugin directory NOT found on disk"
        fi

        # Diagnose: check FRESH WirePlumber logs (only since last restart, not stale hours-old logs)
        local wp_bt_log
        wp_bt_log=$(run_as_user "journalctl --user -u wireplumber --no-pager --since '2 minutes ago' 2>/dev/null" | grep -i "bluez\|bluetooth\|spa.*blue\|monitor.*blue\|error\|warn\|fail\|config" | tail -15 || echo "")
        if [[ -n "$wp_bt_log" ]]; then
            warn "WirePlumber Bluetooth log entries:"
            echo "$wp_bt_log" | while IFS= read -r line; do info "    $line"; done
        else
            warn "No Bluetooth mentions in WirePlumber journal — SPA plugin may not be loading"
        fi

        # Diagnose: check if WirePlumber config is actually being read
        local wp_conf_check=""
        if [[ -f /etc/wireplumber/wireplumber.conf.d/51-soundsync.conf ]]; then
            wp_conf_check="/etc/wireplumber/wireplumber.conf.d/51-soundsync.conf"
        elif [[ -f /etc/wireplumber/bluetooth.lua.d/51-soundsync.lua ]]; then
            wp_conf_check="/etc/wireplumber/bluetooth.lua.d/51-soundsync.lua"
        fi
        if [[ -n "$wp_conf_check" ]]; then
            info "WP config file exists: $wp_conf_check"
            info "  Content: $(head -3 "$wp_conf_check" 2>/dev/null)"
        fi

        # Diagnose: check WP's actual config search paths
        info "Checking WirePlumber config search paths..."
        local wp_binary_path
        wp_binary_path=$(run_as_user "which wireplumber 2>/dev/null" || echo "unknown")
        info "  WP binary: $wp_binary_path"

        # Check all possible Lua config locations
        for confdir in \
            /usr/share/wireplumber/bluetooth.lua.d \
            /etc/wireplumber/bluetooth.lua.d \
            "$(eval echo "~${RUN_USER}")/.config/wireplumber/bluetooth.lua.d"; do
            if [[ -d "$confdir" ]]; then
                info "  Config dir exists: $confdir/"
                ls -la "$confdir"/ 2>/dev/null | while IFS= read -r l; do info "    $l"; done
            fi
        done

        # Check if the default WP bluetooth config exists at all
        local default_bt_lua
        default_bt_lua=$(find /usr/share/wireplumber -name "50-bluez-config.lua" -o -name "50-bluez-monitor.lua" 2>/dev/null | head -1)
        if [[ -n "$default_bt_lua" ]]; then
            info "  Default BT config: $default_bt_lua"
        else
            warn "  No default WP bluetooth Lua config found in /usr/share/wireplumber/"
            info "  This may mean WP was installed without Bluetooth support compiled"
            info "  Try: apt install --reinstall wireplumber pipewire-module-bluetooth"
        fi

        # Check WP full journal output to see startup errors
        info "Full WP journal since last restart:"
        run_as_user "journalctl --user -u wireplumber --no-pager --since '2 minutes ago' 2>/dev/null" | tail -20 | while IFS= read -r l; do info "    $l"; done

        # Try aggressive recovery: restart WirePlumber one more time
        info "Attempting WirePlumber restart for BlueZ5 recovery..."
        run_as_user "systemctl --user restart wireplumber" 2>/dev/null || true
        sleep 5
        bluez_device=$(run_as_user "pw-cli list-objects 2>/dev/null" | grep -c "device.api.*=.*bluez5\|api.bluez5" | tr -dc '0-9' || echo "0")
        bluez_device=${bluez_device:-0}
        if [[ "$bluez_device" -gt 0 ]]; then
            ok "WirePlumber BlueZ5 monitor active after re-restart ($bluez_device device(s))"
            ACTIONS+=("Re-restarted WirePlumber to activate BlueZ5 monitor")
        else
            if bluetoothctl show &>/dev/null; then
                FAILURES+=("WirePlumber BlueZ5 monitor not active despite BT adapter present")
            fi
        fi
    fi

    # ── E10: Final verification output (same commands user would run) ────────
    section "Verification Output"
    info "Running verification commands as user '$RUN_USER':"

    # First verify pactl is working at all
    if ! run_as_user "pactl info >/dev/null 2>&1"; then
        fail "pactl cannot connect to PipeWire — audio commands will fail"
        info "  Checking if pipewire-pulse is running..."
        run_as_user "systemctl --user status pipewire-pulse 2>&1" | head -5 | while IFS= read -r l; do info "    $l"; done
        info "  Checking PULSE_SERVER / socket..."
        info "    /run/user/${RUN_UID}/pulse/native exists: $(test -S /run/user/${RUN_UID}/pulse/native && echo yes || echo NO)"
        info "    /run/user/${RUN_UID}/pipewire-0 exists:   $(test -S /run/user/${RUN_UID}/pipewire-0 && echo yes || echo NO)"
    fi
    echo ""

    # 1. BlueZ5 in PipeWire
    echo -e "  ${BOLD}$ pw-cli list-objects | grep bluez5${NC}"
    local v_bluez
    v_bluez=$(run_as_user "pw-cli list-objects 2>&1" | grep -i "bluez5" || true)
    if [[ -n "$v_bluez" ]]; then
        echo "$v_bluez" | head -10 | while IFS= read -r line; do echo "    $line"; done
    else
        echo -e "    ${RED}(no bluez5 entries — WirePlumber BlueZ monitor not active)${NC}"
    fi
    echo ""

    # 2. Default sink
    echo -e "  ${BOLD}$ pactl get-default-sink${NC}"
    local v_sink
    v_sink=$(run_as_user "pactl get-default-sink 2>&1" || true)
    if [[ -n "$v_sink" ]]; then
        echo "    $v_sink"
    else
        echo -e "    ${RED}(could not query default sink)${NC}"
    fi
    echo ""

    # 3. Bluetooth adapter
    echo -e "  ${BOLD}$ bluetoothctl show | grep -E 'Class|Name|Alias|Discoverable'${NC}"
    local v_bt
    v_bt=$(bluetoothctl show 2>/dev/null | grep -E "Class:|Name:|Alias:|Discoverable:" || true)
    if [[ -n "$v_bt" ]]; then
        echo "$v_bt" | while IFS= read -r line; do echo "    $line"; done
    else
        echo -e "    ${RED}(no Bluetooth adapter found)${NC}"
    fi
    echo ""

    # 4. PipeWire links
    echo -e "  ${BOLD}$ pw-link -l | grep soundsync${NC}"
    local v_links
    v_links=$(run_as_user "pw-link -l 2>&1" | grep -i "soundsync" || true)
    if [[ -n "$v_links" ]]; then
        echo "$v_links" | head -15 | while IFS= read -r line; do echo "    $line"; done
    else
        echo -e "    ${YELLOW}(no soundsync links — service may still be starting)${NC}"
    fi
    echo ""

    # 5. Bluetooth audio sources
    echo -e "  ${BOLD}$ pactl list short sources | grep bluez_input${NC}"
    local v_bt_src
    v_bt_src=$(run_as_user "pactl list short sources 2>&1" | grep "bluez_input" || true)
    if [[ -n "$v_bt_src" ]]; then
        echo "$v_bt_src" | while IFS= read -r line; do echo "    $line"; done
    else
        echo -e "    ${YELLOW}(none — pair and play from a phone to see BT audio nodes)${NC}"
    fi
    echo ""

    # 6. All sinks
    echo -e "  ${BOLD}$ pactl list short sinks${NC}"
    local v_sinks
    v_sinks=$(run_as_user "pactl list short sinks 2>&1" || true)
    if [[ -n "$v_sinks" ]]; then
        echo "$v_sinks" | while IFS= read -r line; do echo "    $line"; done
    else
        echo -e "    ${RED}(no sinks — PipeWire may not be running)${NC}"
    fi
    echo ""

    # 7. SoundSync modules
    echo -e "  ${BOLD}$ pactl list short modules | grep soundsync${NC}"
    local v_mods
    v_mods=$(run_as_user "pactl list short modules 2>&1" | grep "soundsync" || true)
    if [[ -n "$v_mods" ]]; then
        echo "$v_mods" | while IFS= read -r line; do echo "    $line"; done
    else
        echo -e "    ${YELLOW}(no soundsync modules — service creates these on start)${NC}"
    fi
    echo ""
}

# ═══════════════════════════════════════════════════════════════════════════════
# SECTION F: RUN REPAIRS (unless --diagnose-only)
# ═══════════════════════════════════════════════════════════════════════════════
if ! $DIAGNOSE_ONLY && [[ ${#ISSUES[@]} -gt 0 ]]; then
    repair_system

    # ── Post-repair validation ───────────────────────────────────────────────
    section "Post-Repair Validation"

    # Re-check critical pipeline state
    post_sinks=$(run_as_user "pactl list short sinks 2>/dev/null" || echo "")
    post_default=$(run_as_user "pactl get-default-sink 2>/dev/null" || echo "unknown")
    post_objects=$(run_as_user "pw-cli list-objects 2>/dev/null" || echo "")

    # Verify null sink
    if echo "$post_sinks" | grep -q "soundsync-capture"; then
        ok "POST: soundsync-capture null sink present"
    else
        fail "POST: soundsync-capture null sink MISSING"
        FAILURES+=("soundsync-capture not created after repair")
    fi

    # Verify default sink routing
    if echo "$post_default" | grep -q "soundsync-capture\|effect_input.soundsync-eq"; then
        ok "POST: Default sink routed to SoundSync ($post_default)"
    else
        warn "POST: Default sink is '$post_default' — may need manual correction"
    fi

    # Verify BlueZ5 in PipeWire
    post_bluez5=$(echo "$post_objects" | grep -c "device.api.*=.*bluez5\|api.bluez5" | tr -dc '0-9' || echo "0")
    post_bluez5=${post_bluez5:-0}
    if [[ "$post_bluez5" -gt 0 ]]; then
        ok "POST: WirePlumber BlueZ5 monitor active"
    else
        warn "POST: BlueZ5 not yet in PipeWire — may take a few seconds after WP restart"
    fi

    # Verify BT adapter
    post_bt=$(bluetoothctl show 2>/dev/null || echo "")
    if echo "$post_bt" | grep -q "Powered: yes"; then
        ok "POST: Bluetooth adapter powered"
    else
        fail "POST: Bluetooth adapter NOT powered"
    fi
    if echo "$post_bt" | grep -q "Discoverable: yes"; then
        ok "POST: Bluetooth adapter discoverable"
    else
        fail "POST: Bluetooth adapter NOT discoverable"
    fi

    # Verify parec capture
    if pgrep -f "parec" &>/dev/null; then
        ok "POST: Audio capture (parec) running"
    else
        info "POST: parec not running (SoundSync may still be initializing)"
    fi

    # Verify PipeWire links
    post_links=$(run_as_user "pw-link -l 2>/dev/null" || echo "")
    if echo "$post_links" | grep -q "soundsync-capture.*parec\|parec.*soundsync-capture"; then
        ok "POST: parec linked to soundsync-capture.monitor"
    elif [[ "$SVC_SOUNDSYNC" == "active" ]]; then
        info "POST: Waiting for SoundSync to establish capture links..."
    fi

    # Save post-repair state
    echo "$post_sinks" > "$REPORT_DIR/post-repair-sinks.txt"
    echo "$post_objects" > "$REPORT_DIR/post-repair-pw-objects.txt"
    echo "$post_links" > "$REPORT_DIR/post-repair-pw-links.txt"
    echo "$post_bt" > "$REPORT_DIR/post-repair-bt-show.txt"

elif $DIAGNOSE_ONLY; then
    info "Skipping repairs (--diagnose-only)"
elif [[ ${#ISSUES[@]} -eq 0 ]]; then
    info "No issues detected — skipping repairs"
fi

# ═══════════════════════════════════════════════════════════════════════════════
# SECTION G: FAILURE DUMP (if issues remain after repair)
# ═══════════════════════════════════════════════════════════════════════════════
if [[ ${#FAILURES[@]} -gt 0 ]]; then
    section "Failure Diagnostics Dump"
    info "Collecting detailed diagnostics for ${#FAILURES[@]} remaining failures..."

    run_as_user "pw-dump" > "$REPORT_DIR/post-repair-pw-dump.json" 2>/dev/null || true
    run_as_user "pactl list" > "$REPORT_DIR/post-repair-pactl-list.txt" 2>/dev/null || true
    run_as_user "pw-cli list-objects" > "$REPORT_DIR/post-repair-pw-objects.txt" 2>/dev/null || true
    bluetoothctl show > "$REPORT_DIR/post-repair-bt-show.txt" 2>/dev/null || true
    run_as_user "systemctl --user status pipewire wireplumber pipewire-pulse" > "$REPORT_DIR/post-repair-user-services.txt" 2>&1 || true
    systemctl status bluetooth soundsync > "$REPORT_DIR/post-repair-system-services.txt" 2>&1 || true
    journalctl -u soundsync --no-pager -n 100 > "$REPORT_DIR/soundsync-journal.txt" 2>/dev/null || true
    journalctl --user -u pipewire --no-pager -n 50 > "$REPORT_DIR/pipewire-journal.txt" 2>/dev/null || true
    journalctl --user -u wireplumber --no-pager -n 50 > "$REPORT_DIR/wireplumber-journal.txt" 2>/dev/null || true

    echo ""
    echo -e "${RED}${BOLD}Remaining failures:${NC}"
    for f in "${FAILURES[@]}"; do
        echo -e "  ${RED}*${NC} $f"
    done
    echo ""
    echo "Full diagnostics saved to: $REPORT_DIR/"
    echo "Share with: tar czf soundsync-doctor.tar.gz -C /tmp $(basename "$REPORT_DIR")"
fi

# ═══════════════════════════════════════════════════════════════════════════════
# SECTION H: SUMMARY
# ═══════════════════════════════════════════════════════════════════════════════
echo ""
echo -e "${CYAN}${BOLD}═══════════════════════════════════════${NC}"
echo -e "${CYAN}${BOLD}  SoundSync Doctor — Summary${NC}"
echo -e "${CYAN}${BOLD}═══════════════════════════════════════${NC}"
echo -e "  Issues Found:    ${#ISSUES[@]}"
echo -e "  Actions Taken:   ${#ACTIONS[@]}"
echo -e "  Remaining:       ${#FAILURES[@]}"
echo ""

# Re-check key systems for final status
final_status() {
    local name="$1" check="$2"
    if eval "$check" &>/dev/null; then
        echo -e "  ${GREEN}[OK]${NC}   $name"
    else
        echo -e "  ${RED}[FAIL]${NC} $name"
    fi
}

final_status "PipeWire running"          "run_as_user 'systemctl --user is-active pipewire'"
final_status "WirePlumber running"       "run_as_user 'systemctl --user is-active wireplumber'"
final_status "PipeWire-Pulse running"    "run_as_user 'systemctl --user is-active pipewire-pulse'"
final_status "Bluetooth active"          "systemctl is-active bluetooth"
final_status "SoundSync active"          "systemctl is-active soundsync"
final_status "BT adapter discoverable"   "bluetoothctl show 2>/dev/null | grep -q 'Discoverable: yes'"
final_status "libspa-0.2-bluetooth"      "find /usr/lib -path '*/spa-0.2/bluez5' -type d 2>/dev/null | grep -q ."
final_status "WP A2DP config (correct fmt)" "{ $WP_IS_05 && find /etc/wireplumber/wireplumber.conf.d -name '51-soundsync*.conf' 2>/dev/null | grep -q .; } || { ! $WP_IS_05 && { find /etc/wireplumber/bluetooth.lua.d -name '51-soundsync*.lua' 2>/dev/null | grep -q . || find $(eval echo ~${RUN_USER})/.config/wireplumber/bluetooth.lua.d -name '51-soundsync*.lua' 2>/dev/null | grep -q .; }; }"
final_status "No conflicting audio svcs" "! pgrep -x pulseaudio &>/dev/null && ! pgrep -f bluealsa &>/dev/null"
final_status "Default sink → SoundSync" "run_as_user 'pactl get-default-sink 2>/dev/null' | grep -q 'soundsync\|effect_input'"
final_status "BlueZ5 in PipeWire"       "run_as_user 'pw-cli list-objects 2>/dev/null' | grep -q 'bluez5'"
final_status "XDG_RUNTIME_DIR exists"    "test -d /run/user/${RUN_UID}"
final_status "DBus session socket"       "test -S /run/user/${RUN_UID}/bus"

echo -e "${CYAN}${BOLD}═══════════════════════════════════════${NC}"
echo ""
echo "Report saved to: $REPORT_DIR/"
echo ""

# ═══════════════════════════════════════════════════════════════════════════════
# JSON OUTPUT (if --json)
# ═══════════════════════════════════════════════════════════════════════════════
if $JSON_OUTPUT; then
    # Build JSON arrays
    json_array() {
        local arr=("$@")
        echo -n "["
        local first=true
        for item in "${arr[@]}"; do
            $first || echo -n ","
            first=false
            # Escape quotes in item
            item="${item//\\/\\\\}"
            item="${item//\"/\\\"}"
            echo -n "\"$item\""
        done
        echo -n "]"
    }

    JSON_FILE="$REPORT_DIR/summary.json"
    cat > "$JSON_FILE" << JSONEOF
{
  "timestamp": "$(date -Iseconds)",
  "user": "$RUN_USER",
  "uid": $RUN_UID,
  "issues_found": $(json_array "${ISSUES[@]+"${ISSUES[@]}"}"),
  "actions_taken": $(json_array "${ACTIONS[@]+"${ACTIONS[@]}"}"),
  "failures_remaining": $(json_array "${FAILURES[@]+"${FAILURES[@]}"}"),
  "services": {
    "pipewire": "$(run_as_user 'systemctl --user is-active pipewire' 2>/dev/null || echo 'inactive')",
    "wireplumber": "$(run_as_user 'systemctl --user is-active wireplumber' 2>/dev/null || echo 'inactive')",
    "pipewire_pulse": "$(run_as_user 'systemctl --user is-active pipewire-pulse' 2>/dev/null || echo 'inactive')",
    "bluetooth": "$(systemctl is-active bluetooth 2>/dev/null || echo 'inactive')",
    "soundsync": "$(systemctl is-active soundsync 2>/dev/null || echo 'inactive')"
  },
  "bluetooth": {
    "powered": $(bluetoothctl show 2>/dev/null | grep -q "Powered: yes" && echo true || echo false),
    "discoverable": $(bluetoothctl show 2>/dev/null | grep -q "Discoverable: yes" && echo true || echo false),
    "pairable": $(bluetoothctl show 2>/dev/null | grep -q "Pairable: yes" && echo true || echo false),
    "class": "$(bluetoothctl show 2>/dev/null | grep -oP 'Class: \K\S+' || echo '')",
    "spa_plugin_installed": $BT_SPA_INSTALLED,
    "a2dp_config_present": $WP_A2DP_CONFIG
  },
  "pipeline": {
    "null_sink": $PIPE_NULL_SINK,
    "eq_sink": $PIPE_EQ_SINK,
    "default_sink": "$(run_as_user 'pactl get-default-sink' 2>/dev/null || echo "$PIPE_DEFAULT_SINK")",
    "bluez5_in_pipewire": $(run_as_user 'pw-cli list-objects 2>/dev/null' | grep -q 'bluez5' && echo true || echo false),
    "capture_running": $(pgrep -f 'parec' &>/dev/null && echo true || echo false)
  },
  "report_dir": "$REPORT_DIR"
}
JSONEOF
    echo "JSON summary: $JSON_FILE"
fi

# Exit code: 0 if no failures remain, 1 if issues persist
if [[ ${#FAILURES[@]} -gt 0 ]]; then
    exit 1
fi
exit 0
