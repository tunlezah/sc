use tracing::info;
use zbus::interface;
use zvariant::ObjectPath;

/// Bluetooth pairing agent that auto-accepts all pairing requests.
/// Registered at `/org/soundsync/agent` with capability "NoInputNoOutput".
pub struct SoundSyncAgent {
    pub auto_pair: bool,
}

#[interface(name = "org.bluez.Agent1")]
impl SoundSyncAgent {
    async fn release(&self) -> zbus::fdo::Result<()> {
        info!("Agent released");
        Ok(())
    }

    async fn request_pin_code(&self, device: ObjectPath<'_>) -> zbus::fdo::Result<String> {
        info!("PIN code requested for {}", device);
        Ok("0000".into())
    }

    async fn request_passkey(&self, device: ObjectPath<'_>) -> zbus::fdo::Result<u32> {
        info!("Passkey requested for {}", device);
        Ok(0)
    }

    async fn request_confirmation(
        &self,
        device: ObjectPath<'_>,
        passkey: u32,
    ) -> zbus::fdo::Result<()> {
        info!("Confirmation requested for {} with passkey {}", device, passkey);
        if self.auto_pair {
            Ok(())
        } else {
            Err(zbus::fdo::Error::NotSupported(
                "Auto-pair disabled".to_string(),
            ))
        }
    }

    async fn request_authorization(&self, device: ObjectPath<'_>) -> zbus::fdo::Result<()> {
        info!("Authorization requested for {}", device);
        if self.auto_pair {
            Ok(())
        } else {
            Err(zbus::fdo::Error::NotSupported(
                "Auto-pair disabled".to_string(),
            ))
        }
    }

    async fn authorize_service(
        &self,
        device: ObjectPath<'_>,
        uuid: String,
    ) -> zbus::fdo::Result<()> {
        info!("Service authorization for {} UUID {}", device, uuid);
        Ok(())
    }

    async fn cancel(&self) -> zbus::fdo::Result<()> {
        info!("Agent cancel");
        Ok(())
    }
}

/// Register the agent on the given D-Bus connection.
pub async fn register_agent(
    connection: &zbus::Connection,
    auto_pair: bool,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let agent = SoundSyncAgent { auto_pair };
    let agent_path = crate::bluetooth::constants::AGENT_PATH;

    connection
        .object_server()
        .at(agent_path, agent)
        .await?;

    // Call AgentManager1.RegisterAgent and RequestDefaultAgent
    let proxy = zbus::Proxy::new(
        connection,
        "org.bluez",
        "/org/bluez",
        "org.bluez.AgentManager1",
    )
    .await?;

    let path = ObjectPath::try_from(agent_path)?;
    proxy
        .call_method("RegisterAgent", &(path.clone(), "NoInputNoOutput"))
        .await?;
    info!("Agent registered at {}", agent_path);

    proxy
        .call_method("RequestDefaultAgent", &(path,))
        .await?;
    info!("Agent set as default");

    Ok(())
}
