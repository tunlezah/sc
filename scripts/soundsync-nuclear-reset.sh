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
monitor.bluez.properties = {
    bluez5.roles = [ a2dp_sink ]
    bluez5.codecs = [ sbc aac ldac aptx aptx_hd ]
    bluez5.enable-sbc-xq = true
    bluez5.enable-hw-volume = true
}
CONF
    ok "Wrote WP 0.5+ config"
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
step 6 "Remove systemd overrides and unmask services"
# ─────────────────────────────────────────────────────────────────────────────
rm -rf ~/.config/systemd/user/wireplumber.service.d
rm -rf ~/.config/systemd/user/pipewire.service.d
systemctl --user unmask pulseaudio.service 2>/dev/null || true
systemctl --user unmask pulseaudio.socket 2>/dev/null || true
systemctl --user daemon-reload
ok "Systemd overrides cleared"

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
WIREPLUMBER_DEBUG=4 timeout 3 wireplumber --version 2>&1 | grep -i bluez | head -3 || true
# Check if WP started by systemd loaded bluez (look at pw-dump)
PW_BLUEZ=$(pw-dump 2>/dev/null | grep -c "bluez5" || echo "0")
echo "    pw-dump bluez5 refs: $PW_BLUEZ"
echo ""

echo -e "${GREEN}${BOLD}Nuclear reset complete.${NC}"
echo "Connect your phone to '$(bluetoothctl show 2>/dev/null | grep -oP 'Alias: \K.*' || echo SoundSync)' and play music."
echo "Open the web UI and click 'Listen' to hear audio in the browser."
