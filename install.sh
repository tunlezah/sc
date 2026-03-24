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
VERSION="2.1.0"
VERSION_FILE="${INSTALL_DIR}/.soundsync-version"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

log()    { echo -e "${GREEN}[SoundSync]${NC} $*"; }
warn()   { echo -e "${YELLOW}[WARNING]${NC} $*"; }
error()  { echo -e "${RED}[ERROR]${NC} $*"; exit 1; }

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
        pipewire pipewire-pulse wireplumber \
        libdbus-1-dev libpipewire-0.3-dev libspa-0.2-dev \
        libclang-dev libopus-dev libmp3lame-dev pkg-config build-essential \
        avahi-daemon avahi-utils \
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
}

# -------------------------------------------------------------------
# 3b. Configure Avahi (mDNS for Chromecast/AirPlay discovery)
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

    # Create service user if needed
    if ! id "${SERVICE_USER}" &>/dev/null; then
        useradd --system --shell /usr/sbin/nologin --groups audio,bluetooth "${SERVICE_USER}"
        log "Created user ${SERVICE_USER}"
    else
        usermod -aG audio,bluetooth "${SERVICE_USER}" 2>/dev/null || true
    fi

    cat > "${SERVICE_FILE}" << EOF
[Unit]
Description=SoundSync Bluetooth Audio Receiver
After=bluetooth.service pipewire.service avahi-daemon.service

[Service]
Type=simple
User=${SERVICE_USER}
Group=audio
WorkingDirectory=${INSTALL_DIR}
ExecStart=${INSTALL_DIR}/soundsync
Environment=XDG_RUNTIME_DIR=/run/user/$(id -u "${SERVICE_USER}" 2>/dev/null || echo "1000")
Environment=RUST_LOG=info
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
EOF

    systemctl daemon-reload
    log "Service file created at ${SERVICE_FILE}"
}

# -------------------------------------------------------------------
# 6. XDG_RUNTIME_DIR setup
# -------------------------------------------------------------------
setup_xdg() {
    log "Setting up XDG_RUNTIME_DIR..."
    loginctl enable-linger "${SERVICE_USER}" 2>/dev/null || warn "Could not enable-linger for ${SERVICE_USER}"
    local uid
    uid=$(id -u "${SERVICE_USER}" 2>/dev/null || echo "1000")
    mkdir -p "/run/user/${uid}"
    chown "${SERVICE_USER}:${SERVICE_USER}" "/run/user/${uid}" 2>/dev/null || true
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
    configure_avahi
    build_soundsync
    create_service
    setup_xdg

    # Write version file
    echo "${VERSION}" > "${VERSION_FILE}"

    # Restart service if it was running before upgrade
    if [[ "${SERVICE_WAS_RUNNING:-false}" == "true" ]]; then
        log "Restarting SoundSync service..."
        systemctl start soundsync || warn "Failed to restart SoundSync service"
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
    log "Web UI available at:"
    log "  http://$(hostname -I 2>/dev/null | awk '{print $1}' || echo 'localhost'):8080"
    log ""
}

main "$@"
