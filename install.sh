#!/usr/bin/env bash
# SoundSync Installer
# Installs SoundSync on Debian/Ubuntu/Raspberry Pi OS.
# Usage: sudo bash install.sh
set -euo pipefail

INSTALL_DIR="/opt/soundsync"
SERVICE_USER="soundsync"
SERVICE_FILE="/etc/systemd/system/soundsync.service"
REPO_DIR="$(cd "$(dirname "$0")" && pwd)"
PREBUILT_BINARY="${REPO_DIR}/soundsync"
RELEASE_BINARY="${REPO_DIR}/target/release/soundsync"
WEBUI_DIST="${REPO_DIR}/webui/dist"
NODE_VERSION="22"
VERSION="1.1.0"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

log()    { echo -e "${GREEN}[SoundSync]${NC} $*"; }
warn()   { echo -e "${YELLOW}[WARNING]${NC} $*"; }
error()  { echo -e "${RED}[ERROR]${NC} $*"; exit 1; }

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
        libclang-dev libopus-dev pkg-config build-essential \
        git curl

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
Class = 0x24043C
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
# 4. Build SoundSync
# -------------------------------------------------------------------
build_soundsync() {
    log "Building SoundSync..."

    # Strategy: use prebuilt binary if found, otherwise compile from source
    if [[ -x "${PREBUILT_BINARY}" ]]; then
        log "Found prebuilt binary at ${PREBUILT_BINARY}"
        mkdir -p "${INSTALL_DIR}"
        cp "${PREBUILT_BINARY}" "${INSTALL_DIR}/soundsync"
    elif [[ -x "${RELEASE_BINARY}" ]]; then
        log "Found release binary at ${RELEASE_BINARY}"
        mkdir -p "${INSTALL_DIR}"
        cp "${RELEASE_BINARY}" "${INSTALL_DIR}/soundsync"
    else
        log "No prebuilt binary found. Compiling from source..."
        cd "${REPO_DIR}"
        cargo build --release
        mkdir -p "${INSTALL_DIR}"
        cp "${RELEASE_BINARY}" "${INSTALL_DIR}/soundsync"
    fi

    chmod +x "${INSTALL_DIR}/soundsync"
    log "Binary installed to ${INSTALL_DIR}/soundsync"

    # Build frontend if not already built
    if [[ ! -d "${WEBUI_DIST}" ]]; then
        log "Building frontend..."
        cd "${REPO_DIR}/webui"
        npm ci
        npm run build
    fi

    # Copy frontend assets
    if [[ -d "${WEBUI_DIST}" ]]; then
        mkdir -p "${INSTALL_DIR}/webui"
        cp -r "${WEBUI_DIST}" "${INSTALL_DIR}/webui/"
        log "Frontend installed to ${INSTALL_DIR}/webui/dist"
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
After=bluetooth.service pipewire.service

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
    log "SoundSync Installer v${VERSION}"
    log "================================"

    if [[ $EUID -ne 0 ]]; then
        error "This script must be run as root. Use: sudo bash install.sh"
    fi

    detect_system
    install_dependencies
    configure_bluetooth
    build_soundsync
    create_service
    setup_xdg

    log ""
    log "================================"
    log "SoundSync installed successfully!"
    log ""
    log "Start the service:"
    log "  sudo systemctl start soundsync"
    log "  sudo systemctl enable soundsync"
    log ""
    log "Check status:"
    log "  sudo systemctl status soundsync"
    log ""
    log "Web UI available at:"
    log "  http://$(hostname -I 2>/dev/null | awk '{print $1}' || echo 'localhost'):8080"
    log ""
}

main "$@"
