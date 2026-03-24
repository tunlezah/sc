use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::process::Command;
use tokio::sync::{mpsc, Mutex};
use tokio::task::JoinHandle;
use tracing::{debug, error, info, warn};

use crate::state::{AppStateHandle, SystemEvent};

const NULL_SINK_NAME: &str = "soundsync-capture";
const RAOP_DISCOVER_MODULE: &str = "module-raop-discover";
const AVAHI_SERVICE: &str = "_raop._tcp";

/// Commands sent from the web API to the AirPlay manager.
#[derive(Debug)]
pub enum AirPlayCommand {
    Discover,
    Connect { name: String },
    Disconnect,
    SetVolume { level: f32 },
}

/// Information about a discovered AirPlay device.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AirPlayDeviceInfo {
    pub name: String,
    pub address: String,
    pub port: u16,
    pub model: String,
}

/// Active AirPlay session state.
struct AirPlaySession {
    device_info: AirPlayDeviceInfo,
    sink_name: String,
    link_ids: Vec<String>,
    monitor_task: JoinHandle<()>,
}

/// Manages AirPlay device discovery and audio routing via PipeWire RAOP modules.
///
/// AirPlay integration leverages PipeWire's built-in RAOP (Remote Audio Output Protocol)
/// support. Discovery uses Avahi to find `_raop._tcp` services. Audio routing is done
/// via `pw-link` to connect the SoundSync capture sink to the RAOP sink, which means
/// AirPlay output gets EQ-processed audio natively through the PipeWire graph.
///
/// This approach matches the project's existing subprocess pattern for PipeWire
/// interaction (see `pipeline.rs`, `filter_chain.rs`).
pub struct AirPlayManager {
    state: AppStateHandle,
    discovered_devices: Arc<Mutex<HashMap<String, AirPlayDeviceInfo>>>,
    active_session: Option<AirPlaySession>,
    raop_module_loaded: bool,
    raop_available: bool,
}

impl AirPlayManager {
    pub async fn new(state: AppStateHandle) -> Self {
        let raop_available = check_raop_availability().await;
        if !raop_available {
            warn!("PipeWire RAOP module not available. AirPlay features will be limited.");
            info!("Install pipewire-module-raop-sink for full AirPlay support");
        }

        let avahi_running = check_avahi_running().await;
        if !avahi_running {
            warn!("Avahi daemon not running. AirPlay device discovery may not work.");
            info!("Start avahi-daemon: sudo systemctl start avahi-daemon");
        }

        Self {
            state,
            discovered_devices: Arc::new(Mutex::new(HashMap::new())),
            active_session: None,
            raop_module_loaded: false,
            raop_available,
        }
    }

    /// Run the AirPlay manager, processing commands from the channel.
    pub async fn run(mut self, mut cmd_rx: mpsc::Receiver<AirPlayCommand>) {
        info!(
            "AirPlay manager started (RAOP available: {})",
            self.raop_available
        );
        while let Some(cmd) = cmd_rx.recv().await {
            match cmd {
                AirPlayCommand::Discover => {
                    self.handle_discover().await;
                }
                AirPlayCommand::Connect { name } => {
                    self.handle_connect(&name).await;
                }
                AirPlayCommand::Disconnect => {
                    self.handle_disconnect().await;
                }
                AirPlayCommand::SetVolume { level } => {
                    self.handle_set_volume(level).await;
                }
            }
        }
        info!("AirPlay manager shutting down");
        self.handle_disconnect().await;
        self.unload_raop_module().await;
    }

    /// Discover AirPlay devices using Avahi and PipeWire RAOP module.
    async fn handle_discover(&mut self) {
        info!("Starting AirPlay device discovery");

        // Ensure RAOP discover module is loaded for PipeWire to find devices
        if self.raop_available && !self.raop_module_loaded {
            self.load_raop_module().await;
        }

        // Use avahi-browse to find AirPlay devices
        let devices = self.discovered_devices.clone();
        let state = self.state.clone();

        tokio::spawn(async move {
            let discovered = discover_airplay_devices().await;
            let mut devs = devices.lock().await;

            for device in discovered {
                info!(
                    "Discovered AirPlay device: {} at {}:{}",
                    device.name, device.address, device.port
                );
                devs.insert(device.name.clone(), device.clone());
                state.publish(SystemEvent::AirPlayDeviceDiscovered { device });
            }

            if devs.is_empty() {
                info!("No AirPlay devices found on the network");
            }
        });
    }

    /// Connect to an AirPlay device by routing audio through PipeWire.
    async fn handle_connect(&mut self, name: &str) {
        // Disconnect any existing session first
        self.handle_disconnect().await;

        let device_info = {
            let devs = self.discovered_devices.lock().await;
            match devs.get(name) {
                Some(d) => d.clone(),
                None => {
                    warn!("AirPlay device not found: {}", name);
                    self.state.publish(SystemEvent::AirPlayError {
                        message: format!("Device not found: {}", name),
                    });
                    return;
                }
            }
        };

        info!(
            "Connecting to AirPlay device: {} at {}:{}",
            device_info.name, device_info.address, device_info.port
        );

        // Ensure RAOP module is loaded
        if self.raop_available && !self.raop_module_loaded {
            self.load_raop_module().await;
        }

        // Find the RAOP sink in PipeWire that matches this device
        let sink_name = match find_raop_sink(&device_info).await {
            Some(name) => name,
            None => {
                // Try loading a specific RAOP sink for this device
                match load_raop_sink(&device_info).await {
                    Ok(name) => name,
                    Err(e) => {
                        error!(
                            "Failed to find or create RAOP sink for {}: {}",
                            device_info.name, e
                        );
                        self.state.publish(SystemEvent::AirPlayError {
                            message: format!(
                                "Cannot create audio route to {}: {}",
                                device_info.name, e
                            ),
                        });
                        return;
                    }
                }
            }
        };

        // Create pw-link connections from the capture monitor to the RAOP sink
        let link_ids = match create_pw_links(NULL_SINK_NAME, &sink_name).await {
            Ok(ids) => ids,
            Err(e) => {
                error!("Failed to create PipeWire links to {}: {}", sink_name, e);
                self.state.publish(SystemEvent::AirPlayError {
                    message: format!("Audio routing failed: {}", e),
                });
                return;
            }
        };

        info!(
            "AirPlay connected: {} via sink {} ({} links)",
            device_info.name,
            sink_name,
            link_ids.len()
        );

        // Start a monitor task to verify the link stays active
        let monitor_sink = sink_name.clone();
        let monitor_state = self.state.clone();
        let monitor_device_name = device_info.name.clone();
        let monitor_task = tokio::spawn(async move {
            run_airplay_monitor(monitor_sink, monitor_state, monitor_device_name).await;
        });

        self.active_session = Some(AirPlaySession {
            device_info: device_info.clone(),
            sink_name,
            link_ids,
            monitor_task,
        });

        // Update state
        {
            let mut app = self.state.state.write().await;
            app.airplay_active = Some(device_info.name.clone());
        }

        self.state.publish(SystemEvent::AirPlaySessionStarted {
            device: device_info,
        });
    }

    /// Disconnect from the active AirPlay session.
    async fn handle_disconnect(&mut self) {
        if let Some(session) = self.active_session.take() {
            info!("Disconnecting AirPlay: {}", session.device_info.name);

            session.monitor_task.abort();

            // Remove PipeWire links
            for link_id in &session.link_ids {
                remove_pw_link(link_id).await;
            }

            let device_name = session.device_info.name.clone();

            // Update state
            {
                let mut app = self.state.state.write().await;
                app.airplay_active = None;
            }

            self.state
                .publish(SystemEvent::AirPlaySessionStopped { device_name });
        }
    }

    /// Set volume on the active AirPlay device via PipeWire.
    async fn handle_set_volume(&self, level: f32) {
        if let Some(ref session) = self.active_session {
            let volume = level.clamp(0.0, 1.0);
            let sink_name = session.sink_name.clone();

            let result = Command::new("pactl")
                .args([
                    "set-sink-volume",
                    &sink_name,
                    &format!("{}%", (volume * 100.0) as u32),
                ])
                .output()
                .await;

            match result {
                Ok(output) if output.status.success() => {
                    debug!("AirPlay volume set to {:.0}%", volume * 100.0);
                }
                Ok(output) => {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    warn!("Failed to set AirPlay volume: {}", stderr);
                }
                Err(e) => {
                    warn!("Failed to run pactl for volume: {}", e);
                }
            }
        }
    }

    /// Load the PipeWire RAOP discover module.
    async fn load_raop_module(&mut self) {
        let result = Command::new("pactl")
            .args(["load-module", RAOP_DISCOVER_MODULE])
            .output()
            .await;

        match result {
            Ok(output) if output.status.success() => {
                self.raop_module_loaded = true;
                info!("RAOP discover module loaded");
            }
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                // Module might already be loaded
                if stderr.contains("Module already loaded") || stderr.contains("already loaded") {
                    self.raop_module_loaded = true;
                    debug!("RAOP discover module already loaded");
                } else {
                    warn!("Failed to load RAOP discover module: {}", stderr);
                }
            }
            Err(e) => {
                warn!("Failed to run pactl to load RAOP module: {}", e);
            }
        }
    }

    /// Unload the PipeWire RAOP discover module.
    async fn unload_raop_module(&mut self) {
        if self.raop_module_loaded {
            let _ = Command::new("pactl")
                .args(["unload-module", RAOP_DISCOVER_MODULE])
                .output()
                .await;
            self.raop_module_loaded = false;
        }
    }
}

/// Check if PipeWire RAOP module is available on the system.
async fn check_raop_availability() -> bool {
    // Check if the RAOP sink module file exists
    let result = Command::new("find")
        .args([
            "/usr/lib",
            "-name",
            "libpipewire-module-raop-sink*",
            "-type",
            "f",
        ])
        .output()
        .await;

    if let Ok(output) = result {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if !stdout.trim().is_empty() {
                return true;
            }
        }
    }

    // Also check common PipeWire module paths
    let paths = [
        "/usr/lib/pipewire-0.3/libpipewire-module-raop-sink.so",
        "/usr/lib/x86_64-linux-gnu/pipewire-0.3/libpipewire-module-raop-sink.so",
        "/usr/lib/aarch64-linux-gnu/pipewire-0.3/libpipewire-module-raop-sink.so",
    ];

    for path in &paths {
        if std::path::Path::new(path).exists() {
            return true;
        }
    }

    false
}

/// Check if the Avahi daemon is running.
async fn check_avahi_running() -> bool {
    let result = Command::new("systemctl")
        .args(["is-active", "--quiet", "avahi-daemon"])
        .output()
        .await;

    matches!(result, Ok(output) if output.status.success())
}

/// Discover AirPlay devices on the network using avahi-browse.
async fn discover_airplay_devices() -> Vec<AirPlayDeviceInfo> {
    let mut devices = Vec::new();

    // Run avahi-browse to find RAOP services
    // Use -t flag to terminate after dumping all currently cached results,
    // combined with a timeout as a safety net
    let result = tokio::time::timeout(
        Duration::from_secs(10),
        Command::new("avahi-browse")
            .args(["-t", "-r", "-p", "-k", AVAHI_SERVICE])
            .output(),
    )
    .await;

    let output = match result {
        Ok(Ok(output)) if output.status.success() => output,
        Ok(Ok(output)) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            // avahi-browse may exit non-zero but still produce output
            if !output.stdout.is_empty() {
                output
            } else {
                warn!("avahi-browse failed: {}", stderr);
                return discover_via_pw_cli().await;
            }
        }
        Ok(Err(e)) => {
            warn!("Failed to run avahi-browse: {}", e);
            return discover_via_pw_cli().await;
        }
        Err(_) => {
            warn!("avahi-browse timed out, trying pw-cli fallback");
            return discover_via_pw_cli().await;
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Parse avahi-browse parseable output format:
    // =;interface;protocol;name;type;domain;hostname;address;port;txt
    let mut seen_names = std::collections::HashSet::new();

    for line in stdout.lines() {
        let fields: Vec<&str> = line.split(';').collect();
        if fields.len() >= 9 && fields[0] == "=" {
            let raw_name = decode_avahi_escapes(fields[3]);
            let name = extract_raop_friendly_name(&raw_name);
            let address = fields[7].to_string();
            let port: u16 = fields[8].parse().unwrap_or(7000);

            // Skip duplicates (same device on different interfaces)
            if seen_names.contains(&name) {
                continue;
            }
            seen_names.insert(name.clone());

            // Extract model from TXT record if available, decode escapes
            let model = if fields.len() > 9 {
                extract_txt_field(fields[9], "am")
                    .map(|m| decode_avahi_escapes(&m))
                    .unwrap_or_else(|| "AirPlay".to_string())
            } else {
                "AirPlay".to_string()
            };

            // Skip IPv6 addresses for simplicity
            if address.contains(':') {
                continue;
            }

            devices.push(AirPlayDeviceInfo {
                name,
                address,
                port,
                model,
            });
        }
    }

    devices
}

/// Fallback discovery via pw-cli when avahi-browse is not available.
async fn discover_via_pw_cli() -> Vec<AirPlayDeviceInfo> {
    let mut devices = Vec::new();

    let result = Command::new("pw-cli").args(["list-objects"]).output().await;

    if let Ok(output) = result {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            // Look for RAOP sinks in PipeWire objects
            let mut current_name = String::new();
            for line in stdout.lines() {
                if line.contains("raop") || line.contains("RAOP") {
                    if line.contains("node.description") || line.contains("node.nick") {
                        if let Some(value) = extract_pw_property(line) {
                            current_name = value;
                        }
                    }
                    if line.contains("node.name") && !current_name.is_empty() {
                        devices.push(AirPlayDeviceInfo {
                            name: current_name.clone(),
                            address: String::new(),
                            port: 7000,
                            model: "AirPlay".to_string(),
                        });
                        current_name.clear();
                    }
                }
            }
        }
    }

    devices
}

/// Extract a field value from an Avahi TXT record string.
fn extract_txt_field(txt: &str, key: &str) -> Option<String> {
    let search = format!("\"{}=", key);
    if let Some(start) = txt.find(&search) {
        let value_start = start + search.len();
        if let Some(end) = txt[value_start..].find('"') {
            return Some(txt[value_start..value_start + end].to_string());
        }
    }
    None
}

/// Decode avahi-browse DNS-SD escaped strings.
///
/// In parseable output (`-p` flag), avahi-browse encodes special characters as
/// `\NNN` where NNN is a 3-digit decimal ASCII code. For example:
/// - `\032` = space (ASCII 32)
/// - `\064` = `@` (ASCII 64)
/// - `\043` = `#` (ASCII 43)
fn decode_avahi_escapes(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            // Try to read 3 decimal digits
            let mut digits = String::new();
            let mut pending = Vec::new();
            for _ in 0..3 {
                if let Some(d) = chars.next() {
                    pending.push(d);
                    if d.is_ascii_digit() {
                        digits.push(d);
                    } else {
                        break;
                    }
                } else {
                    break;
                }
            }
            if digits.len() == 3 {
                if let Ok(code) = digits.parse::<u8>() {
                    result.push(code as char);
                } else {
                    // Not a valid u8, emit literally
                    result.push('\\');
                    for ch in pending {
                        result.push(ch);
                    }
                }
            } else {
                // Not 3 digits, emit the backslash and whatever we consumed
                result.push('\\');
                for ch in pending {
                    result.push(ch);
                }
            }
        } else {
            result.push(c);
        }
    }
    result
}

/// Extract the friendly device name from a RAOP service name.
///
/// RAOP service names are typically in the format `MACADDRESS@Device Name`
/// (e.g., `CCCCAC98497B@Lounge Room Stereo`). This extracts just the
/// human-readable part after the `@`. If there is no `@`, returns the
/// full name. Also strips any trailing `#` that some devices append.
fn extract_raop_friendly_name(raw_name: &str) -> String {
    let name = if let Some(at_pos) = raw_name.find('@') {
        raw_name[at_pos + 1..].to_string()
    } else {
        raw_name.to_string()
    };
    // Strip trailing '#' that some AirPlay devices append
    name.trim_end_matches('#').trim().to_string()
}

/// Extract a property value from a pw-cli output line.
fn extract_pw_property(line: &str) -> Option<String> {
    if let Some(eq_pos) = line.find('=') {
        let value = line[eq_pos + 1..].trim().trim_matches('"');
        if !value.is_empty() {
            return Some(value.to_string());
        }
    }
    None
}

/// Find a PipeWire RAOP sink matching the given AirPlay device.
async fn find_raop_sink(device: &AirPlayDeviceInfo) -> Option<String> {
    let result = Command::new("pactl")
        .args(["list", "short", "sinks"])
        .output()
        .await
        .ok()?;

    if !result.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&result.stdout);

    // Look for a sink whose name contains the device address or name
    // RAOP sinks typically have names like "raop_sink.<address>" or contain the device name
    for line in stdout.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() >= 2 {
            let sink_name = fields[1];
            if sink_name.contains("raop") {
                // Match by address
                if !device.address.is_empty() {
                    let addr_underscore = device.address.replace('.', "_");
                    if sink_name.contains(&addr_underscore) || sink_name.contains(&device.address) {
                        return Some(sink_name.to_string());
                    }
                }
                // Match by device name (case-insensitive)
                let device_name_lower = device.name.to_lowercase();
                let sink_lower = sink_name.to_lowercase();
                if sink_lower.contains(&device_name_lower.replace(' ', "_"))
                    || sink_lower.contains(&device_name_lower.replace(' ', ""))
                {
                    return Some(sink_name.to_string());
                }
            }
        }
    }

    None
}

/// Load a RAOP sink for a specific device via pactl.
async fn load_raop_sink(device: &AirPlayDeviceInfo) -> Result<String, String> {
    let sink_name = format!("raop_sink.{}", device.address.replace('.', "_"));

    let result = Command::new("pactl")
        .args([
            "load-module",
            "module-raop-sink",
            &format!("server=[{}]:{}", device.address, device.port),
            &format!("sink_name={}", sink_name),
            &format!("sink_properties=device.description=\"{}\"", device.name),
        ])
        .output()
        .await
        .map_err(|e| format!("Failed to run pactl: {}", e))?;

    if !result.status.success() {
        let stderr = String::from_utf8_lossy(&result.stderr);
        return Err(format!("pactl load-module failed: {}", stderr));
    }

    // Wait briefly for the sink to appear
    tokio::time::sleep(Duration::from_millis(500)).await;

    Ok(sink_name)
}

/// Create PipeWire links from the capture monitor to the RAOP sink.
/// Links both left and right channels (FL and FR).
async fn create_pw_links(source_sink: &str, target_sink: &str) -> Result<Vec<String>, String> {
    let mut link_ids = Vec::new();

    // Link left channel: monitor_FL -> playback_FL
    let channels = [("FL", "FL"), ("FR", "FR")];

    for (source_ch, target_ch) in &channels {
        let source_port = format!("{}:monitor_{}", source_sink, source_ch);
        let target_port = format!("{}:playback_{}", target_sink, target_ch);

        let result = Command::new("pw-link")
            .args([&source_port, &target_port])
            .output()
            .await
            .map_err(|e| format!("Failed to run pw-link: {}", e))?;

        if result.status.success() {
            // pw-link doesn't return an ID, but we track the port pair for removal
            link_ids.push(format!("{}|{}", source_port, target_port));
            info!("Created PipeWire link: {} -> {}", source_port, target_port);
        } else {
            let stderr = String::from_utf8_lossy(&result.stderr);
            // If the link already exists, that's fine
            if stderr.contains("already linked") || stderr.contains("File exists") {
                link_ids.push(format!("{}|{}", source_port, target_port));
                debug!(
                    "PipeWire link already exists: {} -> {}",
                    source_port, target_port
                );
            } else {
                return Err(format!(
                    "pw-link failed for {} -> {}: {}",
                    source_port, target_port, stderr
                ));
            }
        }
    }

    Ok(link_ids)
}

/// Remove a PipeWire link by its ID (source_port|target_port).
async fn remove_pw_link(link_id: &str) {
    let parts: Vec<&str> = link_id.split('|').collect();
    if parts.len() == 2 {
        let result = Command::new("pw-link")
            .args(["-d", parts[0], parts[1]])
            .output()
            .await;

        match result {
            Ok(output) if output.status.success() => {
                debug!("Removed PipeWire link: {} -> {}", parts[0], parts[1]);
            }
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                debug!("pw-link -d warning: {}", stderr);
            }
            Err(e) => {
                warn!("Failed to remove PipeWire link: {}", e);
            }
        }
    }
}

/// Monitor an AirPlay session to detect disconnection.
async fn run_airplay_monitor(sink_name: String, state: AppStateHandle, device_name: String) {
    let mut interval = tokio::time::interval(Duration::from_secs(5));

    loop {
        interval.tick().await;

        // Verify the RAOP sink still exists in PipeWire
        let result = Command::new("pactl")
            .args(["list", "short", "sinks"])
            .output()
            .await;

        match result {
            Ok(output) if output.status.success() => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                if !stdout.contains(&sink_name) {
                    info!("AirPlay sink {} disappeared, session ended", sink_name);
                    state.publish(SystemEvent::AirPlaySessionStopped {
                        device_name: device_name.clone(),
                    });
                    {
                        let mut app = state.state.write().await;
                        app.airplay_active = None;
                    }
                    break;
                }
            }
            _ => {
                // pactl failed, but don't terminate the session for transient errors
                debug!("AirPlay monitor: pactl check failed");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decode_avahi_escapes_space() {
        assert_eq!(decode_avahi_escapes("Hello\\032World"), "Hello World");
    }

    #[test]
    fn test_decode_avahi_escapes_at_and_hash() {
        assert_eq!(
            decode_avahi_escapes("CCCCAC98497B\\064Lounge\\032Room\\032Stereo\\043"),
            "CCCCAC98497B@Lounge Room Stereo#"
        );
    }

    #[test]
    fn test_decode_avahi_escapes_no_escapes() {
        assert_eq!(decode_avahi_escapes("Plain Name"), "Plain Name");
    }

    #[test]
    fn test_decode_avahi_escapes_trailing_backslash() {
        assert_eq!(decode_avahi_escapes("test\\"), "test\\");
    }

    #[test]
    fn test_decode_avahi_escapes_partial_digits() {
        assert_eq!(decode_avahi_escapes("test\\12x"), "test\\12x");
    }

    #[test]
    fn test_extract_raop_friendly_name_with_mac() {
        assert_eq!(
            extract_raop_friendly_name("CCCCAC98497B@Lounge Room Stereo#"),
            "Lounge Room Stereo"
        );
    }

    #[test]
    fn test_extract_raop_friendly_name_no_mac() {
        assert_eq!(
            extract_raop_friendly_name("Living Room Speaker"),
            "Living Room Speaker"
        );
    }

    #[test]
    fn test_extract_raop_friendly_name_no_hash() {
        assert_eq!(
            extract_raop_friendly_name("AABBCCDDEEFF@Kitchen"),
            "Kitchen"
        );
    }

    #[test]
    fn test_full_pipeline() {
        let raw = "CCCCAC98497B\\064Lounge\\032Room\\032Stereo\\043";
        let decoded = decode_avahi_escapes(raw);
        assert_eq!(decoded, "CCCCAC98497B@Lounge Room Stereo#");
        let friendly = extract_raop_friendly_name(&decoded);
        assert_eq!(friendly, "Lounge Room Stereo");
    }
}
