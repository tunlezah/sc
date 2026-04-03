#!/usr/bin/env bash
# Comprehensive SoundSync/WirePlumber/PipeWire diagnostic gatherer
# Run as the normal user (mark), NOT root
set -uo pipefail

echo "=== SoundSync Deep Diagnostic Gatherer ==="
echo "Date: $(date -Iseconds)"
echo "User: $(whoami) (UID $(id -u))"
echo ""

echo "========================================"
echo "1. WIREPLUMBER STATE & CACHE FILES"
echo "========================================"
echo "--- WP state directory ---"
ls -laR ~/.local/state/wireplumber/ 2>/dev/null || echo "(no state dir)"
echo ""
echo "--- WP state database content ---"
for f in ~/.local/state/wireplumber/*.json ~/.local/state/wireplumber/**/*.json; do
    if [[ -f "$f" ]]; then
        echo "FILE: $f ($(wc -c < "$f") bytes)"
        cat "$f" | head -20
        echo "..."
    fi
done 2>/dev/null || echo "(no state files)"
echo ""
echo "--- WP cache ---"
ls -laR ~/.cache/wireplumber/ 2>/dev/null || echo "(no cache dir)"
echo ""

echo "========================================"
echo "2. ALL WIREPLUMBER CONFIG FILES (every location)"
echo "========================================"
for dir in /usr/share/wireplumber /etc/wireplumber ~/.config/wireplumber; do
    echo "--- $dir ---"
    if [[ -d "$dir" ]]; then
        find "$dir" -type f -name "*.lua" -o -name "*.conf" 2>/dev/null | sort | while read f; do
            echo "  $f ($(wc -l < "$f") lines)"
        done
    else
        echo "  (does not exist)"
    fi
done
echo ""

echo "--- Content of ALL soundsync/custom configs ---"
find /etc/wireplumber ~/.config/wireplumber -name "*soundsync*" -o -name "*51-*" 2>/dev/null | while read f; do
    echo "=== $f ==="
    cat "$f"
    echo ""
done

echo "========================================"
echo "3. PIPEWIRE CONFIG & STATE"
echo "========================================"
echo "--- PipeWire config ---"
cat /usr/share/pipewire/pipewire.conf 2>/dev/null | grep -A3 "bluez5" || echo "(no bluez5 in pipewire.conf)"
echo ""
echo "--- PipeWire user config overrides ---"
find ~/.config/pipewire /etc/pipewire -type f 2>/dev/null | sort || echo "(none)"
echo ""
echo "--- PipeWire modules loaded ---"
pw-cli list-objects 2>/dev/null | grep "module.name" || echo "(cannot list)"
echo ""

echo "========================================"
echo "4. BLUETOOTH SYSTEM STATE"
echo "========================================"
echo "--- bluetoothctl show ---"
bluetoothctl show 2>/dev/null
echo ""
echo "--- bluetoothctl devices ---"
bluetoothctl devices 2>/dev/null
echo ""
echo "--- /etc/bluetooth/main.conf ---"
cat /etc/bluetooth/main.conf 2>/dev/null || echo "(missing)"
echo ""
echo "--- D-Bus BlueZ objects ---"
busctl --system tree org.bluez 2>/dev/null || echo "(cannot query)"
echo ""
echo "--- BlueZ registered endpoints (check for competing) ---"
dbus-send --system --print-reply --dest=org.bluez /org/bluez/hci0 \
    org.freedesktop.DBus.Properties.GetAll string:org.bluez.Media1 2>/dev/null | head -20 || echo "(cannot query)"
echo ""

echo "========================================"
echo "5. SYSTEMD USER SERVICES STATE"
echo "========================================"
echo "--- User service list (audio related) ---"
systemctl --user list-units --all 2>/dev/null | grep -iE "pipe|wire|pulse|audio|sound|blue"
echo ""
echo "--- Masked services ---"
systemctl --user list-unit-files --state=masked 2>/dev/null
echo ""
echo "--- WP service overrides ---"
ls -la ~/.config/systemd/user/wireplumber.service.d/ 2>/dev/null || echo "(no overrides)"
cat ~/.config/systemd/user/wireplumber.service.d/*.conf 2>/dev/null || echo "(no override content)"
echo ""
echo "--- PipeWire service overrides ---"
ls -la ~/.config/systemd/user/pipewire*.service.d/ 2>/dev/null || echo "(no PW overrides)"
echo ""

echo "========================================"
echo "6. RUNNING PROCESSES"
echo "========================================"
ps aux | grep -iE "pipewire|wireplumber|pulse|bluez|bluetooth|soundsync" | grep -v grep
echo ""

echo "========================================"
echo "7. SPA PLUGIN DETAILS"
echo "========================================"
echo "--- SPA bluez5 directory ---"
ls -la /usr/lib/x86_64-linux-gnu/spa-0.2/bluez5/ 2>/dev/null || echo "(missing)"
echo ""
echo "--- SPA plugin search path ---"
echo "SPA_PLUGIN_DIR=${SPA_PLUGIN_DIR:-not set}"
echo "SPA_DATA_DIR=${SPA_DATA_DIR:-not set}"
echo "PIPEWIRE_MODULE_DIR=${PIPEWIRE_MODULE_DIR:-not set}"
echo ""
echo "--- ldd on libspa-bluez5.so (check dependencies) ---"
ldd /usr/lib/x86_64-linux-gnu/spa-0.2/bluez5/libspa-bluez5.so 2>/dev/null | grep -E "not found|dbus|blue|glib" || echo "(all deps satisfied)"
echo ""

echo "========================================"
echo "8. WIREPLUMBER VERBOSE STARTUP (5 seconds)"
echo "========================================"
echo "--- Stopping WP ---"
systemctl --user stop wireplumber 2>/dev/null
sleep 1
echo "--- Starting WP manually with full debug ---"
WIREPLUMBER_DEBUG=5 timeout 5 /usr/bin/wireplumber 2>&1 | tee /tmp/wp-full-debug.txt | \
    grep -iE "bluez|bluetooth|blue.*monitor|media.*endpoint|error|warn|fail|cannot|could not|denied|refused|component.*bluetooth" | head -40
echo ""
echo "--- WP debug line count ---"
wc -l /tmp/wp-full-debug.txt
echo "--- Any errors at all ---"
grep -iE "error|fail|cannot|denied|refused|could not" /tmp/wp-full-debug.txt | grep -v "libcamera" | head -20
echo ""
echo "--- Starting WP back via systemd ---"
systemctl --user start wireplumber 2>/dev/null
sleep 2

echo "========================================"
echo "9. DBUS PERMISSIONS"  
echo "========================================"
echo "--- User groups ---"
groups
echo ""
echo "--- D-Bus bluetooth policy files ---"
ls -la /etc/dbus-1/system.d/*bluetooth* /usr/share/dbus-1/system.d/*bluetooth* 2>/dev/null || echo "(none found)"
echo ""
echo "--- D-Bus org.bluez owner ---"
busctl --system status org.bluez 2>/dev/null | head -10
echo ""

echo "========================================"
echo "10. PACKAGE VERSIONS & INTEGRITY"
echo "========================================"
dpkg -l | grep -iE "pipewire|wireplumber|libspa|bluez|bluetooth" | awk '{print $1, $2, $3}'
echo ""
echo "--- Package file verification for libspa-0.2-bluetooth ---"
dpkg -V libspa-0.2-bluetooth 2>/dev/null || echo "(dpkg -V not available or package ok)"
echo ""

echo "========================================"
echo "11. PREVIOUS SOUNDSYNC ARTIFACTS"
echo "========================================"
echo "--- Files modified by SoundSync/doctor ---"
find /etc/wireplumber /etc/bluetooth /etc/dbus-1 ~/.config/wireplumber ~/.config/systemd/user \
    -newer /usr/share/wireplumber/bluetooth.lua.d/50-bluez-config.lua -type f 2>/dev/null | sort
echo ""
echo "--- Backup files ---"
find /etc/bluetooth -name "*.bak*" 2>/dev/null
echo ""

echo "========================================"  
echo "12. KERNEL & HARDWARE"
echo "========================================"
echo "--- Bluetooth kernel modules ---"
lsmod | grep -iE "bluetooth|btusb|bnep|rfcomm"
echo ""
echo "--- hci devices ---"
hciconfig -a 2>/dev/null || echo "(hciconfig not available)"
echo ""

echo "========================================"
echo "DONE. Full WP debug saved to /tmp/wp-full-debug.txt"
echo "========================================"
