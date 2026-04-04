#!/usr/bin/env bash
# SoundSync Installer
# Installs SoundSync on Debian/Ubuntu/Raspberry Pi OS.
# Usage: sudo bash install.sh [--uninstall]
set -euo pipefail

INSTALL_DIR="/opt/soundsync"
SERVICE_USER="soundsync"
SERVICE_FILE="/etc/systemd/system/soundsync.service"
REPO_DIR="$(cd "$(dirname "$0")" && pwd)"
PREBUILT_BINARY="${REPO_DIR}/soundsync"
RELEASE_BINARY="${REPO_DIR}/target/release/soundsync"
WEBUI_DIST="${REPO_DIR}/webui/dist"
NODE_VERSION="22"
VERSION="2.8.3"
VERSION_FILE="${INSTALL_DIR}/.soundsync-version"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

log()    { echo -e "${GREEN}[SoundSync]${NC} $*"; }
warn()   { echo -e "${YELLOW}[WARNING]${NC} $*"; }
error()  { echo -e "${RED}[ERROR]${NC} $*"; exit 1; }

# Wait for a systemd service to become active. Returns 0 on success, 1 on timeout.
wait_for_active() {
    local svc="$1" max="${2:-15}" is_user="${3:-false}"
    for i in $(seq 1 "$max"); do
        if [[ "$is_user" == "true" ]]; then
            if su - "${RUN_USER}" -s /bin/bash -c "systemctl --user is-active '$svc'" &>/dev/null; then
                return 0
            fi
        else
            if systemctl is-active "$svc" &>/dev/null; then
                return 0
            fi
        fi
        sleep 1
    done
    return 1
}

# -------------------------------------------------------------------
# Uninstall
# -------------------------------------------------------------------
uninstall_soundsync() {
    log "SoundSync Uninstaller v${VERSION}"
    log "================================"

    if [[ $EUID -ne 0 ]]; then
        error "This script must be run as root. Use: sudo bash install.sh --uninstall"
    fi

    log "Stopping SoundSync service..."
    systemctl stop soundsync 2>/dev/null || true
    systemctl disable soundsync 2>/dev/null || true

    log "Removing service file..."
    rm -f "${SERVICE_FILE}"
    systemctl daemon-reload

    log "Removing install directory (${INSTALL_DIR})..."
    rm -rf "${INSTALL_DIR}"

    log "Removing service user..."
    if id "${SERVICE_USER}" &>/dev/null; then
        userdel "${SERVICE_USER}" 2>/dev/null || warn "Could not remove user ${SERVICE_USER}"
    fi

    log ""
    log "================================"
    log "SoundSync has been completely uninstalled."
    log ""
    log "Note: System dependencies (bluez, pipewire, etc.) were NOT removed."
    log "      Remove them manually if no longer needed."
    log ""
    exit 0
}

# -------------------------------------------------------------------
# 1. System detection and conflict check
# -------------------------------------------------------------------
detect_system() {
    log "Detecting system..."

    if [[ ! -f /etc/os-release ]]; then
        error "Cannot detect OS. /etc/os-release not found."
    fi

    # shellcheck source=/dev/null
    source /etc/os-release
    log "Detected: ${PRETTY_NAME}"

    if [[ "${ID}" != "debian" && "${ID}" != "ubuntu" && "${ID}" != "raspbian" ]]; then
        warn "Unsupported distribution: ${ID}. Proceeding anyway..."
    fi

    # Check for conflicting Bluetooth agents
    if command -v bluealsa &>/dev/null; then
        warn "bluealsa detected. This may conflict with SoundSync."
        warn "Consider removing it: sudo apt remove bluealsa"
    fi

    if systemctl is-active --quiet bluealsa 2>/dev/null; then
        warn "bluealsa service is running. SoundSync may not work correctly."
    fi

    # Check for snd-aloop
    if lsmod 2>/dev/null | grep -q snd_aloop; then
        warn "snd-aloop kernel module loaded. This may interfere with audio routing."
    fi
}

# -------------------------------------------------------------------
# 2. Install system dependencies
# -------------------------------------------------------------------
install_dependencies() {
    log "Installing system dependencies..."
    apt-get update -qq
    apt-get install -y -qq \
        bluetooth bluez \
        pipewire pipewire-pulse pipewire-audio pipewire-alsa wireplumber pulseaudio-utils \
        bluez-tools \
        libspa-0.2-bluetooth \
        libdbus-1-dev libpipewire-0.3-dev libspa-0.2-dev \
        libclang-dev libopus-dev libmp3lame-dev pkg-config build-essential \
        avahi-daemon avahi-utils \
        ffmpeg \
        git curl unzip

    # Install Rust if not present
    if ! command -v cargo &>/dev/null; then
        log "Installing Rust..."
        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
        # shellcheck source=/dev/null
        source "${HOME}/.cargo/env"
    else
        log "Rust already installed: $(rustc --version)"
    fi

    # Install Node.js if not present
    if ! command -v node &>/dev/null || ! node -v | grep -q "v${NODE_VERSION}"; then
        log "Installing Node.js ${NODE_VERSION}..."
        curl -fsSL https://deb.nodesource.com/setup_${NODE_VERSION}.x | bash -
        apt-get install -y -qq nodejs
    else
        log "Node.js already installed: $(node --version)"
    fi
}

# -------------------------------------------------------------------
# 3. Configure Bluetooth
# -------------------------------------------------------------------
configure_bluetooth() {
    log "Configuring Bluetooth..."

    local BT_CONF="/etc/bluetooth/main.conf"

    if [[ -f "${BT_CONF}" ]]; then
        cp "${BT_CONF}" "${BT_CONF}.bak.$(date +%s)"
    fi

    cat > "${BT_CONF}" << 'BTCONF'
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

    log "Bluetooth configured. Restarting service..."
    systemctl restart bluetooth || warn "Failed to restart bluetooth service"
    if wait_for_active bluetooth 10; then
        log "Bluetooth service verified active"
    else
        warn "Bluetooth service not active after restart — will retry after full setup"
    fi
}

# -------------------------------------------------------------------
# 3b. Configure WirePlumber for A2DP sink role
# -------------------------------------------------------------------
configure_wireplumber() {
    log "Configuring WirePlumber for A2DP audio sink..."

    # WirePlumber 0.5+ uses .conf files
    local WP_CONF_DIR="/etc/wireplumber/wireplumber.conf.d"
    # WirePlumber 0.4.x uses Lua files
    local WP_LUA_DIR="/etc/wireplumber/bluetooth.lua.d"

    # Detect WirePlumber version
    local WP_VERSION
    WP_VERSION=$(wireplumber --version 2>/dev/null | grep -oP '\d+\.\d+' | head -1 || echo "0.4")

    # User config directory (preferred — works without root WirePlumber restart)
    local RUN_USER="${SUDO_USER:-$(whoami)}"
    local USER_HOME
    USER_HOME=$(eval echo "~${RUN_USER}")
    local WP_USER_LUA_DIR="${USER_HOME}/.config/wireplumber/bluetooth.lua.d"

    log "Detected WirePlumber version: ${WP_VERSION}"

    # Clean up wrong-format configs from previous installs
    if dpkg --compare-versions "${WP_VERSION}" ge "0.5" 2>/dev/null; then
        # WP 0.5+: remove stale Lua configs
        rm -f "${WP_LUA_DIR}/51-soundsync.lua" 2>/dev/null || true
        rm -f "${WP_USER_LUA_DIR}/51-soundsync-a2dp.lua" 2>/dev/null || true
    else
        # WP 0.4.x: remove stale .conf configs (WP 0.4.x ignores these!)
        rm -f "${WP_CONF_DIR}/51-soundsync.conf" 2>/dev/null || true
    fi

    # Check version FIRST (not directory existence — we may have created the dir ourselves)
    if dpkg --compare-versions "${WP_VERSION}" ge "0.5" 2>/dev/null; then
        # WirePlumber 0.5+ (SPA JSON config)
        mkdir -p "${WP_CONF_DIR}"
        cat > "${WP_CONF_DIR}/51-soundsync.conf" << 'WPCONF'
# SoundSync: Enable A2DP sink role so Bluetooth devices can stream audio here
monitor.bluez.properties = {
    bluez5.roles = [ a2dp_sink ]
    bluez5.codecs = [ sbc aac ldac aptx aptx_hd ]
    bluez5.enable-sbc-xq = true
    bluez5.enable-msbc = false
    bluez5.enable-hw-volume = true
    bluez5.a2dp.opus.pro.channels = 0
}
WPCONF
        log "WirePlumber 0.5+ config written to ${WP_CONF_DIR}/51-soundsync.conf"
    else
        # WirePlumber 0.4.x (Lua config)
        # Write to user config dir (preferred)
        mkdir -p "${WP_USER_LUA_DIR}"
        cat > "${WP_USER_LUA_DIR}/51-soundsync-a2dp.lua" << 'WPLUA'
-- SoundSync: Enable A2DP sink role so Bluetooth devices can stream audio here
-- IMPORTANT: Modify individual properties — do NOT replace the entire
-- bluez_monitor.properties table, as that wipes defaults like with-logind.
bluez_monitor.properties["bluez5.roles"] = "[ a2dp_sink ]"
bluez_monitor.properties["bluez5.codecs"] = "[ sbc aac ldac aptx aptx_hd ]"
bluez_monitor.properties["bluez5.enable-sbc-xq"] = true
bluez_monitor.properties["bluez5.enable-msbc"] = false
bluez_monitor.properties["bluez5.enable-hw-volume"] = true
WPLUA
        chown -R "${RUN_USER}:${RUN_USER}" "${USER_HOME}/.config/wireplumber" 2>/dev/null || true
        log "WirePlumber 0.4.x user config written to ${WP_USER_LUA_DIR}/51-soundsync-a2dp.lua"

        # Also write to /etc/ as fallback
        mkdir -p "${WP_LUA_DIR}"
        cat > "${WP_LUA_DIR}/51-soundsync.lua" << 'WPLUA'
-- SoundSync: Enable A2DP sink role (fallback system-wide config)
-- IMPORTANT: Modify individual properties — do NOT replace the entire
-- bluez_monitor.properties table, as that wipes defaults like with-logind.
bluez_monitor.properties["bluez5.roles"] = "[ a2dp_sink ]"
bluez_monitor.properties["bluez5.codecs"] = "[ sbc aac ldac aptx aptx_hd ]"
bluez_monitor.properties["bluez5.enable-sbc-xq"] = true
bluez_monitor.properties["bluez5.enable-msbc"] = false
bluez_monitor.properties["bluez5.enable-hw-volume"] = true
WPLUA
        log "WirePlumber 0.4.x fallback config written to ${WP_LUA_DIR}/51-soundsync.lua"
    fi

    # Restart WirePlumber to pick up the new config
    local RUN_USER="${SUDO_USER:-$(whoami)}"
    local RUN_UID
    RUN_UID=$(id -u "${RUN_USER}" 2>/dev/null || echo "1000")

    log "Restarting WirePlumber to apply A2DP config..."
    su - "${RUN_USER}" -s /bin/bash -c "
        export XDG_RUNTIME_DIR=/run/user/${RUN_UID}
        export DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/${RUN_UID}/bus
        systemctl --user restart wireplumber 2>/dev/null
    " || warn "Could not restart WirePlumber via systemctl"

    # Verify WirePlumber actually restarted
    if wait_for_active wireplumber 10 true; then
        log "WirePlumber verified active after config change"
    else
        warn "WirePlumber not active — attempting recovery..."
        # Ensure runtime dir exists
        mkdir -p "/run/user/${RUN_UID}"
        chown "${RUN_USER}:${RUN_USER}" "/run/user/${RUN_UID}" 2>/dev/null || true
        # Try again with full environment
        su - "${RUN_USER}" -s /bin/bash -c "
            export XDG_RUNTIME_DIR=/run/user/${RUN_UID}
            export DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/${RUN_UID}/bus
            systemctl --user daemon-reload
            systemctl --user restart pipewire.service
            sleep 2
            systemctl --user restart wireplumber.service
        " 2>/dev/null || warn "WirePlumber recovery failed — run soundsync-doctor.sh after install"
    fi
}

# -------------------------------------------------------------------
# 3c. Configure Avahi (mDNS for Chromecast/AirPlay discovery)
# -------------------------------------------------------------------
configure_avahi() {
    log "Configuring Avahi (mDNS) for device discovery..."

    # Ensure avahi-daemon is enabled and running
    systemctl enable avahi-daemon 2>/dev/null || warn "Could not enable avahi-daemon"
    systemctl start avahi-daemon 2>/dev/null || warn "Could not start avahi-daemon"

    # Verify avahi is running
    if systemctl is-active --quiet avahi-daemon 2>/dev/null; then
        log "Avahi daemon is running"
    else
        warn "Avahi daemon is not running. Chromecast/AirPlay discovery may not work."
    fi

    # Check for PipeWire RAOP module
    if find /usr/lib -name "libpipewire-module-raop-sink*" -type f 2>/dev/null | grep -q .; then
        log "PipeWire RAOP module found (AirPlay support available)"
    else
        warn "PipeWire RAOP module not found. AirPlay output may be unavailable."
        warn "Try: apt install pipewire-module-raop (package name varies by distro)"
    fi
}

# -------------------------------------------------------------------
# 4. Build SoundSync
# -------------------------------------------------------------------

# Check if a directory contains a valid webui build.
# A valid build must have index.html AND compiled assets (JS files).
# This prevents accidentally deploying the unbuilt source index.html
# which references /src/main.tsx instead of bundled assets.
webui_dist_is_valid() {
    local dir="$1"
    [[ -d "${dir}" ]] && [[ -f "${dir}/index.html" ]] && \
        find "${dir}" -maxdepth 2 -name '*.js' -print -quit 2>/dev/null | grep -q .
}

# Search all known locations for a valid webui dist.
# Sets FOUND_WEBUI_DIST to the path if found, empty otherwise.
find_webui_dist() {
    FOUND_WEBUI_DIST=""

    local candidates=(
        "${REPO_DIR}/soundsync-webui"     # CI artifact download (checked first)
        "${REPO_DIR}/webui/dist"          # built in source tree
        "${REPO_DIR}/dist"                # alternate flat layout
        "${INSTALL_DIR}/webui/dist"       # already installed (lowest priority)
    )

    for dir in "${candidates[@]}"; do
        if webui_dist_is_valid "${dir}"; then
            FOUND_WEBUI_DIST="${dir}"
            return 0
        fi
    done
    return 1
}

# Try to extract webui from a ZIP file if one exists.
# Returns 0 and sets FOUND_WEBUI_DIST on success.
try_extract_webui_zip() {
    local zip_candidates=(
        "${REPO_DIR}/soundsync-webui.zip"
        "${REPO_DIR}/webui.zip"
        "${REPO_DIR}/webui/dist.zip"
    )

    for zipfile in "${zip_candidates[@]}"; do
        if [[ -f "${zipfile}" ]]; then
            log "Found webui archive: ${zipfile}"
            local tmp_extract
            tmp_extract="$(mktemp -d)"
            if unzip -qo "${zipfile}" -d "${tmp_extract}" 2>/dev/null; then
                # The zip may contain a top-level dist/ folder or files directly
                if webui_dist_is_valid "${tmp_extract}/dist"; then
                    mkdir -p "${WEBUI_DIST}"
                    cp -r "${tmp_extract}/dist/"* "${WEBUI_DIST}/"
                elif webui_dist_is_valid "${tmp_extract}"; then
                    mkdir -p "${WEBUI_DIST}"
                    cp -r "${tmp_extract}/"* "${WEBUI_DIST}/"
                else
                    rm -rf "${tmp_extract}"
                    continue
                fi
                rm -rf "${tmp_extract}"
                FOUND_WEBUI_DIST="${WEBUI_DIST}"
                return 0
            fi
            rm -rf "${tmp_extract}"
        fi
    done
    return 1
}

build_soundsync() {
    log "Building SoundSync..."

    # ----- Stop running service before overwriting binary (avoids "Text file busy") -----
    SERVICE_WAS_RUNNING=false
    if systemctl is-active --quiet soundsync 2>/dev/null; then
        log "Stopping running SoundSync service before upgrade..."
        systemctl stop soundsync
        SERVICE_WAS_RUNNING=true
    fi

    # ----- Server binary -----
    # Strategy: use prebuilt binary if found, then check install dir, otherwise compile
    if [[ -x "${PREBUILT_BINARY}" ]]; then
        log "Found prebuilt binary at ${PREBUILT_BINARY}"
        mkdir -p "${INSTALL_DIR}"
        cp "${PREBUILT_BINARY}" "${INSTALL_DIR}/soundsync"
    elif [[ -x "${RELEASE_BINARY}" ]]; then
        log "Found release binary at ${RELEASE_BINARY}"
        mkdir -p "${INSTALL_DIR}"
        cp "${RELEASE_BINARY}" "${INSTALL_DIR}/soundsync"
    elif [[ -x "${INSTALL_DIR}/soundsync" ]]; then
        log "Binary already installed at ${INSTALL_DIR}/soundsync — skipping build"
    else
        log "No prebuilt binary found. Compiling from source..."
        cd "${REPO_DIR}"
        cargo build --release
        mkdir -p "${INSTALL_DIR}"
        cp "${RELEASE_BINARY}" "${INSTALL_DIR}/soundsync"
    fi

    chmod +x "${INSTALL_DIR}/soundsync"
    log "Binary installed to ${INSTALL_DIR}/soundsync"

    # ----- Frontend (webui) -----
    # 1) Check all directories for an existing valid build
    if find_webui_dist; then
        log "Found existing webui build at ${FOUND_WEBUI_DIST}"
    # 2) Try extracting from a ZIP archive
    elif try_extract_webui_zip; then
        log "Extracted webui from archive to ${FOUND_WEBUI_DIST}"
    # 3) Fall back to building from source
    else
        log "No prebuilt frontend found. Building from source..."
        cd "${REPO_DIR}/webui"
        npm ci
        npm run build
        if webui_dist_is_valid "${WEBUI_DIST}"; then
            FOUND_WEBUI_DIST="${WEBUI_DIST}"
        fi
    fi

    # Copy frontend assets into install directory.
    # Always replace the existing webui/dist to ensure a clean upgrade.
    if [[ -n "${FOUND_WEBUI_DIST:-}" ]] && [[ "${FOUND_WEBUI_DIST}" != "${INSTALL_DIR}/webui/dist" ]]; then
        rm -rf "${INSTALL_DIR}/webui/dist"
        mkdir -p "${INSTALL_DIR}/webui/dist"
        cp -r "${FOUND_WEBUI_DIST}/"* "${INSTALL_DIR}/webui/dist/"
        log "Frontend installed to ${INSTALL_DIR}/webui/dist"
    elif webui_dist_is_valid "${INSTALL_DIR}/webui/dist"; then
        log "Frontend already installed at ${INSTALL_DIR}/webui/dist"
    else
        warn "Frontend build not found. Web UI will not be available."
    fi

    # Copy branding image
    if [[ -f "${REPO_DIR}/StreamCastImage.png" ]]; then
        cp "${REPO_DIR}/StreamCastImage.png" "${INSTALL_DIR}/webui/dist/" 2>/dev/null || true
    fi
}

# -------------------------------------------------------------------
# 5. Create systemd service
# -------------------------------------------------------------------
create_service() {
    log "Creating systemd service..."

    # Use the real user who invoked sudo (not root, not a separate service user).
    # PipeWire and WirePlumber are per-user services — SoundSync MUST run as
    # the same user to share the audio session and see Bluetooth audio nodes.
    local RUN_USER="${SUDO_USER:-$(whoami)}"
    local RUN_UID
    RUN_UID=$(id -u "${RUN_USER}" 2>/dev/null || echo "1000")

    # Ensure user is in audio and bluetooth groups
    usermod -aG audio,bluetooth "${RUN_USER}" 2>/dev/null || true

    # Configure real-time scheduling limits for audio processes.
    # Without these, PipeWire and parec run at SCHED_OTHER (normal priority)
    # and any system activity can preempt audio threads, causing stuttering.
    local LIMITS_FILE="/etc/security/limits.d/99-soundsync-rt.conf"
    cat > "${LIMITS_FILE}" << LIMITSEOF
# Real-time scheduling limits for SoundSync audio
# rtprio 95  = allow SCHED_FIFO/SCHED_RR up to priority 95
# memlock    = prevent audio buffers from being swapped (causes latency)
# nice -15   = allow higher scheduling priority
${RUN_USER}  -  rtprio   95
${RUN_USER}  -  memlock  unlimited
${RUN_USER}  -  nice     -15
@audio       -  rtprio   95
@audio       -  memlock  unlimited
@audio       -  nice     -15
LIMITSEOF
    log "Configured real-time scheduling limits in ${LIMITS_FILE}"

    # PipeWire runs as a systemd user service which does NOT go through PAM,
    # so /etc/security/limits.d/ is never applied. Set DefaultLimitRTPRIO in
    # /etc/systemd/user.conf so PipeWire gets real-time scheduling.
    local USERCONF="/etc/systemd/user.conf"
    if ! grep -q "^DefaultLimitRTPRIO=" "$USERCONF" 2>/dev/null; then
        cat >> "$USERCONF" << 'SYSEOF'

# Real-time scheduling for PipeWire audio (added by SoundSync installer)
DefaultLimitRTPRIO=95
DefaultLimitMEMLOCK=infinity
DefaultLimitNICE=-15
SYSEOF
        log "Configured systemd user service RT limits in ${USERCONF}"
    fi

    cat > "${SERVICE_FILE}" << EOF
[Unit]
Description=SoundSync Bluetooth Audio Receiver
After=bluetooth.service pipewire.service avahi-daemon.service
Wants=pipewire.service wireplumber.service pipewire-pulse.service

[Service]
Type=simple
User=${RUN_USER}
Group=audio
WorkingDirectory=${INSTALL_DIR}
# Real-time scheduling limits (match /etc/security/limits.d/99-soundsync-rt.conf)
LimitRTPRIO=95
LimitMEMLOCK=infinity
LimitNICE=-15
# Clean up orphaned SoundSync PulseAudio modules before starting.
# This prevents duplicate null sinks and loopback modules from accumulating
# across restarts. Safe to run when no modules exist (pactl returns 0).
ExecStartPre=/bin/bash -c 'pactl list short modules 2>/dev/null | grep -E "module-(null-sink|loopback)" | while read -r id name args; do case "\$args" in *soundsync*) pactl unload-module "\$id" 2>/dev/null && echo "Pre-start: unloaded module \$id (\$name)" || true ;; esac; done; true'
ExecStart=${INSTALL_DIR}/soundsync
Environment=XDG_RUNTIME_DIR=/run/user/${RUN_UID}
Environment=DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/${RUN_UID}/bus
Environment=PULSE_RUNTIME_PATH=/run/user/${RUN_UID}/pulse
Environment=RUST_LOG=info
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
EOF

    # Enable linger so PipeWire persists when no login session is active
    loginctl enable-linger "${RUN_USER}" 2>/dev/null || warn "Could not enable-linger for ${RUN_USER}"

    systemctl daemon-reload
    log "Service will run as user '${RUN_USER}' (UID ${RUN_UID})"
    log "Service file created at ${SERVICE_FILE}"
}

# -------------------------------------------------------------------
# 6. XDG_RUNTIME_DIR setup
# -------------------------------------------------------------------
setup_xdg() {
    log "Setting up XDG_RUNTIME_DIR and PipeWire user services..."
    local RUN_USER="${SUDO_USER:-$(whoami)}"
    loginctl enable-linger "${RUN_USER}" 2>/dev/null || warn "Could not enable-linger for ${RUN_USER}"
    local uid
    uid=$(id -u "${RUN_USER}" 2>/dev/null || echo "1000")
    mkdir -p "/run/user/${uid}"
    chown "${RUN_USER}:${RUN_USER}" "/run/user/${uid}" 2>/dev/null || true

    # Ensure PipeWire and WirePlumber are enabled as user services
    # (they must be running for SoundSync to create sinks, capture audio, etc.)
    su - "${RUN_USER}" -s /bin/bash -c "
        export XDG_RUNTIME_DIR=/run/user/${uid}
        export DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/${uid}/bus
        systemctl --user daemon-reload
        systemctl --user enable pipewire.service pipewire-pulse.service wireplumber.service 2>/dev/null
        systemctl --user start pipewire.service pipewire-pulse.service wireplumber.service 2>/dev/null
    " || warn "Could not enable PipeWire user services — they may already be enabled"

    # Verify PipeWire is ready to accept commands
    local pw_ready=false
    for i in $(seq 1 15); do
        if su - "${RUN_USER}" -s /bin/bash -c "
            export XDG_RUNTIME_DIR=/run/user/${uid}
            export DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/${uid}/bus
            pactl info >/dev/null 2>&1
        "; then
            pw_ready=true
            break
        fi
        sleep 1
    done

    if [[ "$pw_ready" == "true" ]]; then
        log "PipeWire verified ready (accepts pactl commands)"
    else
        warn "PipeWire not responding to pactl — SoundSync may need soundsync-doctor.sh"
    fi
}

# -------------------------------------------------------------------
# Main
# -------------------------------------------------------------------
main() {
    # Handle --uninstall flag
    if [[ "${1:-}" == "--uninstall" ]]; then
        uninstall_soundsync
    fi

    log "SoundSync Installer v${VERSION}"
    log "================================"

    if [[ $EUID -ne 0 ]]; then
        error "This script must be run as root. Use: sudo bash install.sh"
    fi

    # Detect upgrade
    if [[ -f "${VERSION_FILE}" ]]; then
        local prev_version
        prev_version="$(cat "${VERSION_FILE}")"
        log "Existing installation detected: v${prev_version}"
        log "Upgrading to: v${VERSION}"
    fi

    detect_system
    install_dependencies
    configure_bluetooth
    configure_wireplumber
    configure_avahi
    build_soundsync
    create_service
    setup_xdg

    # Write version file
    echo "${VERSION}" > "${VERSION_FILE}"

    # Restart service if it was running before upgrade
    if [[ "${SERVICE_WAS_RUNNING:-false}" == "true" ]]; then
        log "Restarting SoundSync service..."
        systemctl start soundsync || warn "Failed to start SoundSync service"
        if wait_for_active soundsync 15; then
            log "SoundSync service verified active"
        else
            warn "SoundSync service not active — check: systemctl status soundsync"
            warn "Try running: sudo bash scripts/soundsync-doctor.sh"
        fi
    fi

    log ""
    log "================================"
    log "SoundSync v${VERSION} installed successfully!"
    log ""
    log "Start the service:"
    log "  sudo systemctl start soundsync"
    log "  sudo systemctl enable soundsync"
    log ""
    log "Check status:"
    log "  sudo systemctl status soundsync"
    log ""
    log "Uninstall:"
    log "  sudo bash install.sh --uninstall"
    log ""
    log "Diagnose issues:"
    log "  sudo bash scripts/soundsync-doctor.sh"
    log ""
    log "Web UI available at:"
    log "  http://$(hostname -I 2>/dev/null | awk '{print $1}' || echo 'localhost'):8080"
    log ""
}

main "$@"
