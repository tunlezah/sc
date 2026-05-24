#!/usr/bin/env bash
# =============================================================================
# soundsync-nuclear-reset.sh — Complete system reset for SoundSync
# =============================================================================
# This script performs a FULL reset of the audio pipeline to a known-good state.
# It does NOT assume any device is connected or any service is running.
#
# Usage: ./soundsync-nuclear-reset.sh
# (Run as the normal user, NOT root. Will use sudo where needed.)
# =============================================================================
set -uo pipefail

RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'
CYAN='\033[0;36m'; BOLD='\033[1m'; NC='\033[0m'

step() { echo -e "\n${CYAN}${BOLD}[$1/$TOTAL] $2${NC}"; }
ok()   { echo -e "  ${GREEN}✓${NC} $*"; }
info() { echo -e "  $*"; }

if [[ $EUID -eq 0 ]]; then
    echo -e "${RED}Do NOT run as root. Run as: ./soundsync-nuclear-reset.sh${NC}"
    exit 1
fi

TOTAL=12
echo -e "${BOLD}SoundSync Nuclear Reset${NC}"
echo "This will completely reset the audio pipeline to a known-good state."
echo ""

# ─────────────────────────────────────────────────────────────────────────────
step 1 "Stop all SoundSync and audio services"
# ─────────────────────────────────────────────────────────────────────────────
sudo systemctl stop soundsync 2>/dev/null || true
systemctl --user stop wireplumber pipewire-pulse pipewire 2>/dev/null || true
sleep 2
# Kill any stragglers
pkill -f "pipewire-filter-chain" 2>/dev/null || true
pkill -f "pw-loopback" 2>/dev/null || true
pkill -f "parec.*soundsync" 2>/dev/null || true
ok "Services stopped"

# ─────────────────────────────────────────────────────────────────────────────
step 2 "Kill competing soundsync system user processes"
# ─────────────────────────────────────────────────────────────────────────────
if id soundsync &>/dev/null; then
    sudo loginctl disable-linger soundsync 2>/dev/null || true
    sudo loginctl terminate-user soundsync 2>/dev/null || true
    sudo pkill -9 -u soundsync 2>/dev/null || true
    sleep 1
    ok "soundsync user processes terminated"
else
    ok "No soundsync system user"
fi

# ─────────────────────────────────────────────────────────────────────────────
step 3 "Remove ALL WirePlumber user state and cache"
# ─────────────────────────────────────────────────────────────────────────────
rm -rf ~/.local/state/wireplumber
rm -rf ~/.local/state/pipewire
rm -rf ~/.cache/wireplumber
ok "State and cache cleared"

# ─────────────────────────────────────────────────────────────────────────────
step 4 "Remove ALL custom WirePlumber configs (user + system)"
# ─────────────────────────────────────────────────────────────────────────────
# User configs
rm -rf ~/.config/wireplumber
ok "Removed ~/.config/wireplumber"

# System configs (need sudo)
sudo rm -rf /etc/wireplumber/wireplumber.conf.d/51-soundsync* 2>/dev/null || true
sudo rm -rf /etc/wireplumber/bluetooth.lua.d/51-soundsync* 2>/dev/null || true
ok "Removed /etc/wireplumber custom configs"

# ─────────────────────────────────────────────────────────────────────────────
step 5 "Write correct WirePlumber A2DP config"
# ─────────────────────────────────────────────────────────────────────────────
WP_VER=$(wireplumber --version 2>/dev/null | grep -oP '\d+\.\d+' | head -1 || echo "0.4")
if dpkg --compare-versions "$WP_VER" ge "0.5" 2>/dev/null; then
    sudo mkdir -p /etc/wireplumber/wireplumber.conf.d
    sudo tee /etc/wireplumber/wireplumber.conf.d/51-soundsync.conf > /dev/null << 'CONF'
# SoundSync — WirePlumber 0.5+ A2DP sink config (written by nuclear-reset)
#
# CRITICAL: monitor.bluez.seat-monitoring = disabled
#   Stock bluez.lua only calls createMonitor() when WpLogind reports
#   seat_state == "active", which requires a session bound to a seat
#   (graphical login or local TTY). SSH-only sessions are "online" but
#   never "active", so without this override the BlueZ monitor never
#   starts on a headless box and no A2DP MediaEndpoints are registered.
wireplumber.profiles = {
    main = {
        monitor.bluez.seat-monitoring = disabled
    }
}

monitor.bluez.properties = {
    bluez5.roles = [ a2dp_sink a2dp_source hfp_hf hfp_ag hsp_hs hsp_ag ]
    bluez5.codecs = [ sbc aac ldac aptx aptx_hd ]
    bluez5.enable-sbc-xq = true
    bluez5.enable-msbc = false
    bluez5.enable-hw-volume = true
}
CONF
    ok "Wrote WP 0.5+ config (with seat-monitoring=disabled — the actual A2DP-on-headless fix)"
else
    sudo mkdir -p /etc/wireplumber/bluetooth.lua.d
    sudo tee /etc/wireplumber/bluetooth.lua.d/51-soundsync.lua > /dev/null << 'LUA'
-- SoundSync A2DP sink config (individual property assignment preserves defaults)
bluez_monitor.properties["bluez5.roles"] = "[ a2dp_sink ]"
bluez_monitor.properties["bluez5.codecs"] = "[ sbc aac ldac aptx aptx_hd ]"
bluez_monitor.properties["bluez5.enable-sbc-xq"] = true
bluez_monitor.properties["bluez5.enable-msbc"] = false
bluez_monitor.properties["bluez5.enable-hw-volume"] = true
LUA
    ok "Wrote WP 0.4.x Lua config"
fi

# ─────────────────────────────────────────────────────────────────────────────
step 6 "Reset systemd overrides + write WirePlumber MDWE override"
# ─────────────────────────────────────────────────────────────────────────────
# Clear stale overrides, then put back the one we actually need:
# MemoryDenyWriteExecute=no on wireplumber.service. Without this, the
# BlueZ5 SPA plugin's codec sub-plugins (LDAC, aptX) fail to mmap
# PROT_WRITE|PROT_EXEC and the bluez monitor never registers endpoints
# when running under systemd. (It works "manually" — that's the tell.)
rm -rf ~/.config/systemd/user/wireplumber.service.d
rm -rf ~/.config/systemd/user/pipewire.service.d
systemctl --user unmask pulseaudio.service 2>/dev/null || true
systemctl --user unmask pulseaudio.socket 2>/dev/null || true

mkdir -p ~/.config/systemd/user/wireplumber.service.d
cat > ~/.config/systemd/user/wireplumber.service.d/override.conf << 'WPOVERRIDE'
# SoundSync: allow libspa-bluez5.so and codec sub-plugins to mmap
# PROT_WRITE|PROT_EXEC. Without this, BlueZ5 SPA fails to load codec
# libraries (LDAC, aptX) and the BlueZ monitor silently does nothing.
[Service]
MemoryDenyWriteExecute=no
WPOVERRIDE
systemctl --user daemon-reload
ok "MDWE override written; stale overrides cleared"

# ─────────────────────────────────────────────────────────────────────────────
step 7 "Fix Bluetooth config"
# ─────────────────────────────────────────────────────────────────────────────
sudo tee /etc/bluetooth/main.conf > /dev/null << 'BT'
[General]
Class = 0x240414
Name = SoundSync
DiscoverableTimeout = 0
PairableTimeout = 0
Discoverable = true
Pairable = true

[Policy]
AutoEnable = true
BT
# Clean up backup spam
sudo find /etc/bluetooth -name "*.bak.*" -delete 2>/dev/null || true
ok "Bluetooth config written, backups cleaned"

# ─────────────────────────────────────────────────────────────────────────────
step 8 "Fix SoundSync service file"
# ─────────────────────────────────────────────────────────────────────────────
RUN_USER="$(whoami)"
RUN_UID="$(id -u)"
sudo tee /etc/systemd/system/soundsync.service > /dev/null << SVCEOF
[Unit]
Description=SoundSync Bluetooth Audio Receiver
After=bluetooth.service pipewire.service avahi-daemon.service
Wants=pipewire.service wireplumber.service pipewire-pulse.service

[Service]
Type=simple
User=${RUN_USER}
Group=audio
WorkingDirectory=/opt/soundsync
ExecStartPre=/bin/bash -c 'pactl list short modules 2>/dev/null | grep -E "module-(null-sink|loopback)" | while read -r id name args; do case "\$args" in *soundsync*) pactl unload-module "\$id" 2>/dev/null || true ;; esac; done; true'
ExecStart=/opt/soundsync/soundsync
Environment=XDG_RUNTIME_DIR=/run/user/${RUN_UID}
Environment=DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/${RUN_UID}/bus
Environment=PULSE_RUNTIME_PATH=/run/user/${RUN_UID}/pulse
Environment=RUST_LOG=info
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
SVCEOF
sudo systemctl daemon-reload
ok "Service file written (User=$RUN_USER)"

# ─────────────────────────────────────────────────────────────────────────────
step 9 "Ensure user permissions and linger"
# ─────────────────────────────────────────────────────────────────────────────
sudo usermod -aG audio,bluetooth "$RUN_USER" 2>/dev/null || true
sudo loginctl enable-linger "$RUN_USER" 2>/dev/null || true
ok "User in audio+bluetooth groups, linger enabled"

# Verify the running user@.service actually picked up the new groups.
# usermod only edits /etc/group; processes already running keep their old
# credential set. WirePlumber inherits from user@.service and silently
# can't register A2DP endpoints if the bluetooth gid is missing.
NEEDS_RELOGIN=false
BT_GID=$(getent group bluetooth 2>/dev/null | cut -d: -f3)
USER_MGR_PID=$(systemctl show -p MainPID --value "user@${RUN_UID}.service" 2>/dev/null)
if [[ -n "$BT_GID" && -n "$USER_MGR_PID" && "$USER_MGR_PID" != "0" && -r "/proc/${USER_MGR_PID}/status" ]]; then
    if ! awk -v g="$BT_GID" '/^Groups:/ { for(i=2;i<=NF;i++) if($i==g){found=1; exit} } END { exit found ? 0 : 1 }' "/proc/${USER_MGR_PID}/status"; then
        echo -e "  ${YELLOW}⚠${NC} user@${RUN_UID}.service does NOT have the bluetooth group yet"
        NEEDS_RELOGIN=true
    fi
fi

# ─────────────────────────────────────────────────────────────────────────────
step 10 "Restart Bluetooth"
# ─────────────────────────────────────────────────────────────────────────────
sudo systemctl restart bluetooth
sleep 2
bluetoothctl power on &>/dev/null || true
bluetoothctl discoverable on &>/dev/null || true
bluetoothctl pairable on &>/dev/null || true
if command -v hciconfig &>/dev/null; then
    sudo hciconfig hci0 class 0x240414 2>/dev/null || true
fi
ok "Bluetooth restarted"

# ─────────────────────────────────────────────────────────────────────────────
step 11 "Start PipeWire stack"
# ─────────────────────────────────────────────────────────────────────────────
systemctl --user start pipewire.service
sleep 1
systemctl --user start pipewire-pulse.service
sleep 1
systemctl --user start wireplumber.service
sleep 3

# Verify PipeWire ready
for i in $(seq 1 10); do pactl info &>/dev/null && break; sleep 1; done
ok "PipeWire stack running"

# ─────────────────────────────────────────────────────────────────────────────
step 12 "Start SoundSync"
# ─────────────────────────────────────────────────────────────────────────────
sudo systemctl start soundsync
sleep 5

# Set default sink
if pactl list short sinks 2>/dev/null | grep -q soundsync-capture; then
    pactl set-default-sink soundsync-capture 2>/dev/null || true
fi
ok "SoundSync started"

# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo -e "${CYAN}${BOLD}═══ Verification ═══${NC}"
echo ""

echo -e "  ${BOLD}Services:${NC}"
for svc in pipewire wireplumber pipewire-pulse; do
    systemctl --user is-active "$svc" &>/dev/null && echo -e "    ${GREEN}✓${NC} $svc" || echo -e "    ${RED}✗${NC} $svc"
done
systemctl is-active bluetooth &>/dev/null && echo -e "    ${GREEN}✓${NC} bluetooth" || echo -e "    ${RED}✗${NC} bluetooth"
systemctl is-active soundsync &>/dev/null && echo -e "    ${GREEN}✓${NC} soundsync" || echo -e "    ${RED}✗${NC} soundsync"

echo ""
echo -e "  ${BOLD}Audio pipeline:${NC}"
pactl list short sinks 2>/dev/null | while read -r l; do echo "    $l"; done
echo ""
echo -e "  ${BOLD}Default sink:${NC} $(pactl get-default-sink 2>/dev/null)"
echo ""
echo -e "  ${BOLD}Bluetooth:${NC}"
bluetoothctl show 2>/dev/null | grep -E "Name:|Class:|Discoverable:|Pairable:" | while read -r l; do echo "    $l"; done
echo ""
echo -e "  ${BOLD}Web UI:${NC} http://$(hostname -I 2>/dev/null | awk '{print $1}'):8080"
echo ""
echo -e "  ${BOLD}WP BlueZ5:${NC}"
# Authoritative test: does BlueZ now advertise A2DP Sink in SDP?
# If yes, the WP bluez monitor successfully registered MediaEndpoints
# with BlueZ. If no, A2DP is broken and devices won't see a speaker.
if bluetoothctl show 2>/dev/null | grep -q '0000110b-0000-1000-8000-00805f9b34fb'; then
    echo -e "    ${GREEN}✓${NC} Adapter advertises Audio Sink (0x110b) — A2DP sink LIVE"
else
    echo -e "    ${RED}✗${NC} Adapter does NOT advertise Audio Sink (0x110b)"
    echo -e "      WirePlumber bluez monitor has not registered any MediaEndpoint."
fi
PW_BLUEZ=$(pw-dump 2>/dev/null | grep -c "bluez5" || echo "0")
echo "    pw-dump bluez5 refs: $PW_BLUEZ"
echo ""

if $NEEDS_RELOGIN; then
    echo -e "${YELLOW}${BOLD}⚠ ACTION REQUIRED:${NC}"
    echo "  The systemd user manager (user@${RUN_UID}.service) does NOT have the"
    echo "  bluetooth/audio groups in its credential set. Group changes only"
    echo "  apply to NEW processes — the user manager and every per-user service"
    echo "  it spawned (PipeWire, WirePlumber) are running with stale credentials."
    echo "  Until this is fixed, WirePlumber cannot register A2DP MediaEndpoints"
    echo "  with BlueZ and devices that pair will not expose an audio sink."
    echo ""
    echo "  Fix by EITHER:"
    echo -e "    (a) ${BOLD}Reboot${NC} the machine (simplest, no impact on SSH session)"
    echo -e "    (b) ${BOLD}sudo loginctl terminate-user ${RUN_USER}${NC} then log back in"
    echo "        (this WILL end your current session)"
    echo ""
fi

echo -e "${GREEN}${BOLD}Nuclear reset complete.${NC}"
echo "Connect your phone to '$(bluetoothctl show 2>/dev/null | grep -oP 'Alias: \K.*' || echo SoundSync)' and play music."
echo "Open the web UI and click 'Listen' to hear audio in the browser."
