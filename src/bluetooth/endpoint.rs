// A2DP endpoint registration is currently disabled — WirePlumber handles
// codec negotiation and transport acquisition natively. This module is
// retained for potential future use (e.g. codec-aware UI updates).
#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::Mutex;

use tracing::info;
use zbus::interface;
use zvariant::{ObjectPath, OwnedValue};

use crate::bluetooth::codecs::AudioCodec;
use crate::state::AppStateHandle;

/// A2DP Media Endpoint implementation for BlueZ.
///
/// BlueZ's `org.bluez.MediaEndpoint1` interface allows us to register as an
/// A2DP sink. One endpoint is created per codec (SBC, AAC, LDAC, aptX, aptX HD)
/// and registered on D-Bus. When a remote device connects and negotiates a codec,
/// BlueZ calls `SelectConfiguration` to let us choose parameters, then
/// `SetConfiguration` to activate the transport.
#[derive(Debug)]
pub struct A2dpEndpoint {
    pub codec: AudioCodec,
    pub state_handle: AppStateHandle,
    pub configuration: Mutex<Option<Vec<u8>>>,
    pub transport_path: Mutex<Option<String>>,
}

#[interface(name = "org.bluez.MediaEndpoint1")]
impl A2dpEndpoint {
    /// Called by BlueZ when a device negotiates this codec and the transport
    /// is ready. `transport` is the D-Bus path of the MediaTransport1 object.
    async fn set_configuration(
        &self,
        transport: ObjectPath<'_>,
        props: HashMap<String, OwnedValue>,
    ) {
        info!(
            codec = %self.codec,
            transport = %transport,
            "A2DP codec configuration set"
        );

        // Store the configuration bytes
        if let Some(config) = props.get("Configuration") {
            if let Ok(owned) = config.try_clone() {
                if let Ok(bytes) = <Vec<u8>>::try_from(owned) {
                    if let Ok(mut guard) = self.configuration.lock() {
                        *guard = Some(bytes);
                    }
                }
            }
        }

        // Store the transport path for later Acquire calls
        let transport_str = transport.to_string();
        if let Ok(mut guard) = self.transport_path.lock() {
            *guard = Some(transport_str.clone());
        }

        // Extract device address from transport path
        // e.g. /org/bluez/hci0/dev_AA_BB_CC_DD_EE_FF/sep1/fd0
        let address = crate::bluetooth::constants::address_from_path(&transport_str)
            .unwrap_or_else(|| "unknown".to_string());

        // Update device state to AudioActive and set codec
        {
            let mut app = self.state_handle.state.write().await;
            if let Some(device) = app.devices.get_mut(&address) {
                device.state = crate::bluetooth::device::DeviceState::AudioActive;
                device.codec = Some(self.codec);
                let dev_name = device.name.clone();
                app.active_device = Some(address.clone());
                info!(
                    "Device {} ({}) is now streaming audio via {:?}",
                    address, dev_name, self.codec
                );
            }
        }

        // Emit StreamStarted event
        self.state_handle
            .publish(crate::state::SystemEvent::StreamStarted {
                address,
                codec: self.codec,
            });
    }

    /// Called by BlueZ to select the best configuration from remote capabilities.
    /// Returns the selected configuration bytes.
    async fn select_configuration(&self, capabilities: Vec<u8>) -> zbus::fdo::Result<Vec<u8>> {
        info!(
            codec = %self.codec,
            caps_len = capabilities.len(),
            "Selecting A2DP configuration from remote capabilities"
        );

        let selected = self.codec.select_configuration(&capabilities);

        if selected.is_empty() {
            Err(zbus::fdo::Error::NotSupported(
                "Capability negotiation failed".into(),
            ))
        } else {
            Ok(selected)
        }
    }

    /// Called by BlueZ when a transport is removed (device disconnected or codec changed).
    async fn clear_configuration(&self, transport: ObjectPath<'_>) {
        let transport_str = transport.to_string();
        info!(
            codec = %self.codec,
            transport = %transport_str,
            "A2DP configuration cleared"
        );

        if let Ok(mut guard) = self.configuration.lock() {
            *guard = None;
        }
        if let Ok(mut guard) = self.transport_path.lock() {
            *guard = None;
        }

        let address = crate::bluetooth::constants::address_from_path(&transport_str)
            .unwrap_or_else(|| "unknown".to_string());

        // Update device state back from AudioActive
        {
            let mut app = self.state_handle.state.write().await;
            if let Some(device) = app.devices.get_mut(&address) {
                device.state = crate::bluetooth::device::DeviceState::Connected;
                device.codec = None;
            }
            if app.active_device.as_deref() == Some(&address) {
                app.active_device = None;
            }
        }

        self.state_handle
            .publish(crate::state::SystemEvent::StreamStopped { address });
    }
}

impl A2dpEndpoint {
    pub fn new(codec: AudioCodec, state_handle: AppStateHandle) -> Self {
        Self {
            codec,
            state_handle,
            configuration: Mutex::new(None),
            transport_path: Mutex::new(None),
        }
    }
}

/// Register all A2DP sink endpoints on the D-Bus connection.
/// This tells BlueZ we can accept audio for each supported codec.
pub async fn register_endpoints(
    connection: &zbus::Connection,
    state: AppStateHandle,
    adapter_name: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let adapter_path = format!("/org/bluez/{}", adapter_name);
    let media_proxy = zbus::Proxy::new(
        connection,
        "org.bluez",
        adapter_path.as_str(),
        "org.bluez.Media1",
    )
    .await?;

    for codec in AudioCodec::all() {
        let endpoint_path = codec.endpoint_path();
        let endpoint = A2dpEndpoint::new(*codec, state.clone());

        // Register the endpoint object on our D-Bus connection
        connection
            .object_server()
            .at(endpoint_path, endpoint)
            .await?;

        // Build the properties dict for RegisterEndpoint
        let mut props: HashMap<&str, zvariant::Value<'_>> = HashMap::new();
        props.insert(
            "UUID",
            zvariant::Value::from(crate::bluetooth::constants::A2DP_SINK_UUID),
        );
        props.insert("Codec", zvariant::Value::from(codec.codec_id()));
        props.insert("Capabilities", zvariant::Value::from(codec.capabilities()));

        let path = ObjectPath::try_from(endpoint_path)?;

        match media_proxy
            .call_method("RegisterEndpoint", &(path, props))
            .await
        {
            Ok(_) => {
                info!(codec = %codec, path = endpoint_path, "Registered A2DP endpoint");
            }
            Err(e) => {
                // Non-fatal: some codecs may not be supported by the adapter
                tracing::warn!(
                    codec = %codec,
                    error = %e,
                    "Failed to register A2DP endpoint (codec may not be supported)"
                );
            }
        }
    }

    Ok(())
}
