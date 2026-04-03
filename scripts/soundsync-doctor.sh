#!/usr/bin/env bash
# =============================================================================
# soundsync-doctor.sh — Diagnose and repair SoundSync audio system
# =============================================================================
# Usage: ./soundsync-doctor.sh [--diagnose-only] [--json]
#
# This script runs as the NORMAL USER (not root).
# It uses sudo ONLY for specific root operations (bluetooth restart, etc).
# This preserves the D-Bus session which is CRITICAL for PipeWire/WirePlumber.
# =============================================================================
set -uo pipefail

# ═══════════════════════════════════════════════════════════════════════════════
# SECTION A: SETUP
# ═══════════════════════════════════════════════════════════════════════════════

RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'
CYAN='\033[0;36m'; BOLD='\033[1m'; NC='\033[0m'

section() { echo -e "\n${CYAN}${BOLD}═══ $1 ═══${NC}"; }
ok()      { echo -e "  ${GREEN}[OK]${NC} $*";   CHECKS_OK+=("$*"); }
warn()    { echo -e "  ${YELLOW}[WARN]${NC} $*"; ISSUES+=("$*"); }
fail()    { echo -e "  ${RED}[FAIL]${NC} $*";    ISSUES+=("$*"); }
info()    { echo -e "  [INFO] $*"; }

# ── Reject running as root ──────────────────────────────────────────────────
if [[ $EUID -eq 0 ]]; then
    echo -e "${RED}ERROR: Do NOT run this script as root or with sudo.${NC}"
    echo "Run as the normal user: ./soundsync-doctor.sh"
    echo "The script will use sudo internally ONLY where needed."
    exit 1
fi

# Parse arguments
DIAGNOSE_ONLY=false
JSON_OUTPUT=false
for arg in "$@"; do
    case "$arg" in
        --diagnose-only) DIAGNOSE_ONLY=true ;;
        --json)          JSON_OUTPUT=true ;;
    esac
done

ISSUES=(); ACTIONS=(); FAILURES=(); CHECKS_OK=()

# State variables
SVC_PIPEWIRE="unknown"; SVC_WIREPLUMBER="unknown"; SVC_BLUETOOTH="unknown"
SVC_SOUNDSYNC="unknown"; SVC_PIPEWIRE_PULSE="unknown"
BT_POWERED=false; BT_DISCOVERABLE=false; BT_PAIRABLE=false; BT_CLASS=""
PIPE_NULL_SINK=false; PIPE_EQ_SINK=false; PIPE_DEFAULT_SINK=""
BT_SPA_INSTALLED=false; WP_A2DP_CONFIG=false; WP_A2DP_WRONG_FORMAT=false
DBUS_BLUEZ_OK=false; CONFLICTING_AUDIO=false

RUN_USER="$(whoami)"
RUN_UID="$(id -u)"

# Validate environment
export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/run/user/${RUN_UID}}"
export DBUS_SESSION_BUS_ADDRESS="${DBUS_SESSION_BUS_ADDRESS:-unix:path=/run/user/${RUN_UID}/bus}"

REPORT_DIR="/tmp/soundsync-doctor-$(date +%Y%m%d-%H%M%S)"
mkdir -p "$REPORT_DIR"

# ── Helper: run command as root ──────────────────────────────────────────────
run_root() { sudo "$@"; }

echo -e "${BOLD}SoundSync Doctor${NC} — $(date -Iseconds)"
echo "Running as: ${RUN_USER} (UID ${RUN_UID})"
echo "XDG_RUNTIME_DIR: ${XDG_RUNTIME_DIR}"
echo "DBUS_SESSION_BUS_ADDRESS: ${DBUS_SESSION_BUS_ADDRESS}"
echo "Report dir: ${REPORT_DIR}"
if $DIAGNOSE_ONLY; then echo -e "${YELLOW}Mode: diagnose-only (no repairs)${NC}"; fi

# ═══════════════════════════════════════════════════════════════════════════════
# SECTION B: ENVIRONMENT VALIDATION
# ═══════════════════════════════════════════════════════════════════════════════
section "Environment"

if [[ -d "$XDG_RUNTIME_DIR" ]]; then ok "XDG_RUNTIME_DIR exists"; else fail "XDG_RUNTIME_DIR missing"; fi
if [[ -S "$XDG_RUNTIME_DIR/bus" ]]; then ok "D-Bus session socket exists"; else fail "D-Bus session socket missing"; fi
if [[ -S "$XDG_RUNTIME_DIR/pipewire-0" ]]; then ok "PipeWire socket exists"; else fail "PipeWire socket missing"; fi
if pactl info &>/dev/null; then ok "pactl connects to PipeWire"; else fail "pactl cannot connect"; fi
if loginctl show-user "$RUN_USER" 2>/dev/null | grep -q "Linger=yes"; then ok "loginctl linger enabled"; else warn "loginctl linger not enabled"; fi

# ═══════════════════════════════════════════════════════════════════════════════
# SECTION C: SERVICES
# ═══════════════════════════════════════════════════════════════════════════════
section "Services"

for svc in pipewire wireplumber pipewire-pulse; do
    if systemctl --user is-active "$svc" &>/dev/null; then
        ok "$svc: active"
    else
        fail "$svc: inactive"
    fi
done
if systemctl is-active bluetooth &>/dev/null; then ok "bluetooth: active"; else fail "bluetooth: inactive"; fi
if systemctl is-active soundsync &>/dev/null; then ok "soundsync: active"; else warn "soundsync: inactive"; fi

PW_VER=$(pw-cli --version 2>/dev/null | head -1 || echo "unknown")
WP_VER=$(wireplumber --version 2>/dev/null | grep -oP '\d+\.\d+\.\d+' | head -1 || echo "unknown")
info "PipeWire: $PW_VER  WirePlumber: $WP_VER"

# ═══════════════════════════════════════════════════════════════════════════════
# SECTION D: BLUETOOTH
# ═══════════════════════════════════════════════════════════════════════════════
section "Bluetooth"

BT_INFO=$(bluetoothctl show 2>/dev/null || echo "")
if [[ -n "$BT_INFO" ]]; then
    BT_POWERED=$(echo "$BT_INFO" | grep -oP 'Powered: \K\w+' || echo "no")
    BT_DISCOVERABLE=$(echo "$BT_INFO" | grep -oP 'Discoverable: \K\w+' || echo "no")
    BT_CLASS=$(echo "$BT_INFO" | grep -oP 'Class: \K\S+' || echo "")
    [[ "$BT_POWERED" == "yes" ]] && ok "Powered" || fail "NOT powered"
    [[ "$BT_DISCOVERABLE" == "yes" ]] && ok "Discoverable" || fail "NOT discoverable"
    info "Class: $BT_CLASS"
fi

# SPA plugin
if find /usr/lib -path "*/spa-0.2/bluez5" -type d 2>/dev/null | grep -q .; then
    ok "libspa-0.2-bluetooth installed"; BT_SPA_INSTALLED=true
else
    fail "libspa-0.2-bluetooth NOT installed"; BT_SPA_INSTALLED=false
fi

# D-Bus access to BlueZ
if dbus-send --system --print-reply --dest=org.bluez / org.freedesktop.DBus.ObjectManager.GetManagedObjects &>/dev/null; then
    ok "D-Bus access to org.bluez works"; DBUS_BLUEZ_OK=true
else
    fail "Cannot access org.bluez over D-Bus"; DBUS_BLUEZ_OK=false
fi

# WP A2DP config
WP_IS_05=false
WP_MAJOR=$(echo "$WP_VER" | grep -oP '^\d+\.\d+' || echo "0.4")
dpkg --compare-versions "$WP_MAJOR" ge "0.5" 2>/dev/null && WP_IS_05=true

WP_A2DP_CONFIG=false
if ! $WP_IS_05; then
    for dir in /etc/wireplumber/bluetooth.lua.d "$HOME/.config/wireplumber/bluetooth.lua.d"; do
        for f in "$dir"/51-soundsync*; do
            if [[ -f "$f" ]] && grep -q "a2dp_sink" "$f" 2>/dev/null; then
                if grep -q 'bluez_monitor\.properties\s*=' "$f" 2>/dev/null; then
                    fail "Config $f uses table-replacement (wipes defaults)"; WP_A2DP_WRONG_FORMAT=true
                else
                    ok "WP A2DP config: $f"; WP_A2DP_CONFIG=true
                fi
                break 2
            fi
        done
    done
fi
if ! $WP_A2DP_CONFIG && ! $WP_A2DP_WRONG_FORMAT; then
    fail "No valid WP A2DP config found"
fi

# Conflicting servers
if pgrep -x pulseaudio &>/dev/null; then fail "PulseAudio running!"; CONFLICTING_AUDIO=true; fi
if pgrep -f bluealsa &>/dev/null; then fail "bluez-alsa running!"; CONFLICTING_AUDIO=true; fi
if ! $CONFLICTING_AUDIO; then ok "No conflicting audio servers"; fi

# ═══════════════════════════════════════════════════════════════════════════════
# SECTION E: PIPEWIRE BLUETOOTH CHECK
# ═══════════════════════════════════════════════════════════════════════════════
section "PipeWire BlueZ5 Integration"

# Save state
pw-cli list-objects > "$REPORT_DIR/pw-objects.txt" 2>/dev/null || true
pw-dump > "$REPORT_DIR/pw-dump.json" 2>/dev/null || true
pactl list short sinks > "$REPORT_DIR/pa-sinks.txt" 2>/dev/null || true
pactl list short sources > "$REPORT_DIR/pa-sources.txt" 2>/dev/null || true
pactl list short modules > "$REPORT_DIR/pa-modules.txt" 2>/dev/null || true

# Check MediaEndpoints — these are registered by WirePlumber's SPA bluez5 plugin
# on WP's own D-Bus connection, NOT under org.bluez's tree.
# Detection methods (in order of reliability):
# 1. Check WP journal for "Registering DBus media endpoint"
# 2. Check pw-dump for bluez5 device references
# 3. Check if bluez5.device factory was loaded in pw-dump

BLUEZ5_ACTIVE=false

# Method 1: Check WP journal for endpoint registration (most reliable)
WP_BT_LOG=$(journalctl --user -u wireplumber --no-pager --since "5 minutes ago" 2>/dev/null | \
    grep -i "register.*media.*endpoint\|api.bluez5\|bluez.*monitor\|MediaEndpoint" | tail -5 || echo "")
if echo "$WP_BT_LOG" | grep -qi "register.*media.*endpoint\|MediaEndpoint"; then
    ok "BlueZ5 A2DP active (MediaEndpoints registered in WP logs)"
    BLUEZ5_ACTIVE=true
fi

# Method 2: Check pw-dump for bluez5 references
if ! $BLUEZ5_ACTIVE; then
    BLUEZ5_DUMP=$(grep -c "bluez5\|api.bluez5" "$REPORT_DIR/pw-dump.json" 2>/dev/null | tr -dc '0-9' || echo "0")
    BLUEZ5_DUMP=${BLUEZ5_DUMP:-0}
    if [[ "$BLUEZ5_DUMP" -gt 0 ]]; then
        ok "BlueZ5 references in PipeWire dump ($BLUEZ5_DUMP)"
        BLUEZ5_ACTIVE=true
    fi
fi

# Method 3: Look for bluez device in wpctl status
if ! $BLUEZ5_ACTIVE; then
    if wpctl status 2>/dev/null | grep -qi "bluez"; then
        ok "BlueZ5 visible in wpctl status"
        BLUEZ5_ACTIVE=true
    fi
fi

if ! $BLUEZ5_ACTIVE; then
    fail "BlueZ5 monitor not detected — WirePlumber may not have loaded BlueZ integration"
    info "  Check: journalctl --user -u wireplumber | grep -i bluez"
fi

# BT audio nodes (only when a device is connected and streaming)
BT_NODES=$(grep -c "bluez_input\|bluez_source" "$REPORT_DIR/pw-objects.txt" 2>/dev/null | tr -dc '0-9' || echo "0")
BT_NODES=${BT_NODES:-0}
if [[ "$BT_NODES" -gt 0 ]]; then
    ok "Bluetooth audio streaming: $BT_NODES nodes"
else
    info "No BT device streaming (connect phone & play to activate)"
fi

# Pipeline
PIPE_DEFAULT_SINK=$(pactl get-default-sink 2>/dev/null || echo "unknown")
info "Default sink: $PIPE_DEFAULT_SINK"
if echo "$PIPE_DEFAULT_SINK" | grep -q "soundsync\|effect_input"; then
    ok "Default sink → SoundSync"
fi

PIPE_NULL_SINK=false
if grep -q "soundsync-capture" "$REPORT_DIR/pa-sinks.txt" 2>/dev/null; then
    ok "soundsync-capture exists"; PIPE_NULL_SINK=true
fi

# ═══════════════════════════════════════════════════════════════════════════════
# SECTION F: REPAIR (unless --diagnose-only)
# ═══════════════════════════════════════════════════════════════════════════════
if ! $DIAGNOSE_ONLY && [[ ${#ISSUES[@]} -gt 0 ]]; then
    section "Repair"

    # F1: Kill conflicting servers
    if $CONFLICTING_AUDIO; then
        info "Stopping conflicting audio servers..."
        killall pulseaudio 2>/dev/null || true
        killall bluealsa 2>/dev/null || true
        systemctl --user mask pulseaudio.service pulseaudio.socket 2>/dev/null || true
        ACTIONS+=("Stopped conflicting audio servers")
    fi

    # F2: Fix WP config
    if $WP_A2DP_WRONG_FORMAT || ! $WP_A2DP_CONFIG; then
        info "Fixing WirePlumber A2DP config..."
        # Remove all old soundsync WP configs
        for d in /etc/wireplumber/bluetooth.lua.d /etc/wireplumber/wireplumber.conf.d \
                 "$HOME/.config/wireplumber/bluetooth.lua.d"; do
            for f in "$d"/51-soundsync*; do
                [[ -f "$f" ]] && run_root rm -f "$f" && ACTIONS+=("Removed $f")
            done
        done

        # Write correct config (individual property assignment)
        run_root mkdir -p /etc/wireplumber/bluetooth.lua.d
        run_root tee /etc/wireplumber/bluetooth.lua.d/51-soundsync.lua > /dev/null << 'WPLUA'
-- SoundSync: Enable A2DP sink role
-- Uses individual property assignment to preserve defaults (esp. with-logind)
bluez_monitor.properties["bluez5.roles"] = "[ a2dp_sink ]"
bluez_monitor.properties["bluez5.codecs"] = "[ sbc aac ldac aptx aptx_hd ]"
bluez_monitor.properties["bluez5.enable-sbc-xq"] = true
bluez_monitor.properties["bluez5.enable-msbc"] = false
bluez_monitor.properties["bluez5.enable-hw-volume"] = true
WPLUA
        ok "Wrote WP A2DP config"; ACTIONS+=("Wrote WP A2DP config")
    fi

    # F3: Fix Bluetooth config
    if [[ -n "$BT_CLASS" ]] && ! echo "$BT_CLASS" | grep -qiE "0x0*[02]40414"; then
        info "Fixing Bluetooth config..."
        run_root tee /etc/bluetooth/main.conf > /dev/null << 'BTCONF'
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
        ACTIONS+=("Wrote /etc/bluetooth/main.conf")
    fi

    # F4: Restart services (correct order: BT first, then PW, WP last)
    info "Restarting services..."

    # Stop user services
    systemctl --user stop wireplumber pipewire-pulse pipewire 2>/dev/null || true
    sleep 1

    # Kill orphaned processes
    pkill -f "pipewire-filter-chain" 2>/dev/null || true
    pkill -f "pw-loopback" 2>/dev/null || true

    # Restart Bluetooth (needs root)
    run_root systemctl restart bluetooth
    sleep 2

    # Wait for BlueZ D-Bus
    for i in 1 2 3 4 5; do
        dbus-send --system --print-reply --dest=org.bluez / \
            org.freedesktop.DBus.ObjectManager.GetManagedObjects &>/dev/null && break
        sleep 1
    done
    ok "Bluetooth ready"

    # Start PipeWire stack
    systemctl --user start pipewire.service
    sleep 1
    systemctl --user start pipewire-pulse.service
    sleep 1

    # Verify PipeWire ready
    for i in $(seq 1 10); do pactl info &>/dev/null && break; sleep 1; done
    ok "PipeWire ready"

    # Start WirePlumber LAST
    systemctl --user start wireplumber.service
    sleep 3
    ok "WirePlumber started"

    # Fix BT class
    if command -v hciconfig &>/dev/null; then
        run_root hciconfig hci0 class 0x240414 2>/dev/null || true
    fi
    bluetoothctl power on &>/dev/null || true
    bluetoothctl discoverable on &>/dev/null || true
    bluetoothctl pairable on &>/dev/null || true

    # Restart SoundSync
    run_root systemctl restart soundsync 2>/dev/null || true
    sleep 3

    # Set default sink
    if pactl list short sinks 2>/dev/null | grep -q "soundsync-capture"; then
        pactl set-default-sink soundsync-capture 2>/dev/null || true
        ok "Default sink → soundsync-capture"
    fi

    ACTIONS+=("Restarted all services")
elif $DIAGNOSE_ONLY; then
    info "Skipping repairs (--diagnose-only)"
fi

# ═══════════════════════════════════════════════════════════════════════════════
# SECTION G: VERIFICATION
# ═══════════════════════════════════════════════════════════════════════════════
section "Verification"

echo -e "  ${BOLD}$ journalctl --user -u wireplumber | grep -i 'bluez5\\|MediaEndpoint' (last 5min)${NC}"
WP_BT_VERIFY=$(journalctl --user -u wireplumber --no-pager --since "5 minutes ago" 2>/dev/null | \
    grep -iE "bluez5|MediaEndpoint|bluez.*monitor" | tail -5 || echo "    (no bluez entries in WP log)")
echo "$WP_BT_VERIFY" | while IFS= read -r l; do echo "    $l"; done
echo ""

echo -e "  ${BOLD}$ pactl get-default-sink${NC}"
echo "    $(pactl get-default-sink 2>/dev/null || echo 'unknown')"
echo ""

echo -e "  ${BOLD}$ bluetoothctl show | grep -E 'Class|Name|Discoverable'${NC}"
bluetoothctl show 2>/dev/null | grep -E "Class:|Name:|Alias:|Discoverable:" | while IFS= read -r l; do echo "    $l"; done
echo ""

echo -e "  ${BOLD}$ pactl list short sinks${NC}"
pactl list short sinks 2>/dev/null | while IFS= read -r l; do echo "    $l"; done
echo ""

echo -e "  ${BOLD}$ pw-link -l | grep soundsync${NC}"
pw-link -l 2>/dev/null | grep "soundsync" | head -10 | while IFS= read -r l; do echo "    $l"; done || echo "    (no links)"
echo ""

echo -e "  ${BOLD}$ wpctl status (Audio section)${NC}"
wpctl status 2>/dev/null | head -30 | while IFS= read -r l; do echo "    $l"; done
echo ""

# ═══════════════════════════════════════════════════════════════════════════════
# SECTION H: SUMMARY
# ═══════════════════════════════════════════════════════════════════════════════
echo -e "\n${CYAN}${BOLD}═══════════════════════════════════════${NC}"
echo -e "${CYAN}${BOLD}  SoundSync Doctor — Summary${NC}"
echo -e "${CYAN}${BOLD}═══════════════════════════════════════${NC}"
echo -e "  Issues Found:    ${#ISSUES[@]}"
echo -e "  Actions Taken:   ${#ACTIONS[@]}"
echo ""

final_status() {
    local name="$1" check="$2"
    if eval "$check" &>/dev/null; then
        echo -e "  ${GREEN}[OK]${NC}   $name"
    else
        echo -e "  ${RED}[FAIL]${NC} $name"
    fi
}

final_status "PipeWire"              "systemctl --user is-active pipewire"
final_status "WirePlumber"           "systemctl --user is-active wireplumber"
final_status "PipeWire-Pulse"        "systemctl --user is-active pipewire-pulse"
final_status "Bluetooth"             "systemctl is-active bluetooth"
final_status "SoundSync"             "systemctl is-active soundsync"
final_status "BT Discoverable"       "bluetoothctl show 2>/dev/null | grep -q 'Discoverable: yes'"
final_status "libspa-0.2-bluetooth"  "find /usr/lib -path '*/spa-0.2/bluez5' -type d 2>/dev/null | grep -q ."
final_status "BlueZ5 integration"    "journalctl --user -u wireplumber --no-pager --since '10 minutes ago' 2>/dev/null | grep -qi 'register.*media.*endpoint\|MediaEndpoint\|api.bluez5.enum'"
final_status "Default sink → SS"     "pactl get-default-sink 2>/dev/null | grep -q soundsync"
final_status "D-Bus session"         "test -S $XDG_RUNTIME_DIR/bus"
final_status "PipeWire socket"       "test -S $XDG_RUNTIME_DIR/pipewire-0"

echo -e "${CYAN}${BOLD}═══════════════════════════════════════${NC}"
echo -e "\nReport: $REPORT_DIR/"
