//! Packet identities for the version being imported, not the running core.
use super::*;
use anyhow::{Context, Result, ensure};
use mc_protocol::codec::{ReadMc, WriteMc};

pub(super) struct Protocol {
    pub version: i32,
    pub known_cb: i32,
    pub known_sb: i32,
    pub registry: i32,
    pub tags: i32,
    pub finish: i32,
    pub hello: i32,
    pub compression: i32,
    pub success: i32,
    pub acknowledged: i32,
    pub disconnect: i32,
}

impl Protocol {
    pub fn canonical() -> Self {
        use mc_protocol::packets::login::{
            LoginAcknowledged, LoginDisconnect, LoginStart, LoginSuccess, SetCompression,
        };
        Self {
            version: mc_protocol::PROTOCOL_VERSION,
            known_cb: ClientboundKnownPacks::ID,
            known_sb: ServerboundKnownPacks::ID,
            registry: RegistryData::ID,
            tags: UpdateTags::ID,
            finish: FinishConfiguration::ID,
            hello: LoginStart::ID,
            compression: SetCompression::ID,
            success: LoginSuccess::ID,
            acknowledged: LoginAcknowledged::ID,
            disconnect: LoginDisconnect::ID,
        }
    }

    pub fn from_cache(root: &Path) -> Result<Self> {
        let metadata: serde_json::Value =
            serde_json::from_slice(&fs::read(root.join("version.json"))?)?;
        let version = metadata["protocol_version"]
            .as_i64()
            .context("missing protocol_version")? as i32;
        ensure!(
            mc_data::protocol_versions::by_protocol(version).is_some(),
            "unsupported capture protocol {version}"
        );
        let packets: serde_json::Value =
            serde_json::from_slice(&fs::read(root.join("reports/packets.json"))?)?;
        let id = |phase: &str, direction: &str, name: &str| -> Result<i32> {
            let value = packets[phase][direction][format!("minecraft:{name}")]["protocol_id"]
                .as_i64()
                .with_context(|| {
                    format!("missing {phase}/{direction}/{name} for protocol {version}")
                })?;
            Ok(i32::try_from(value)?)
        };
        Ok(Self {
            version,
            known_cb: id("configuration", "clientbound", "select_known_packs")?,
            known_sb: id("configuration", "serverbound", "select_known_packs")?,
            registry: id("configuration", "clientbound", "registry_data")?,
            tags: id("configuration", "clientbound", "update_tags")?,
            finish: id("configuration", "clientbound", "finish_configuration")?,
            hello: id("login", "serverbound", "hello")?,
            compression: id("login", "clientbound", "login_compression")?,
            success: id("login", "clientbound", "login_finished")?,
            acknowledged: id("login", "serverbound", "login_acknowledged")?,
            disconnect: id("login", "clientbound", "login_disconnect")?,
        })
    }

    pub async fn login(&self, client: &mut Client, addr: SocketAddr) -> Result<()> {
        use mc_protocol::packets::handshake::{Handshake, NextState};
        client
            .write_packet(&Handshake {
                protocol_version: self.version,
                server_address: "127.0.0.1".into(),
                server_port: addr.port(),
                next_state: NextState::Login,
            })
            .await?;
        let mut hello = Vec::new();
        hello.write_string("RegistryCapture", 16)?;
        hello.write_uuid(uuid::Uuid::nil());
        client.write_raw(self.hello, &hello).await?;
        loop {
            let frame = client.read_frame().await?;
            let mut body = frame.body.as_ref();
            if frame.id == self.compression {
                let threshold = body.read_varint()?;
                ensure!(
                    body.is_empty() && threshold >= 0,
                    "invalid compression threshold"
                );
                client.set_compression(mc_protocol::Compression::Threshold(threshold as usize));
            } else if frame.id == self.success {
                body.read_uuid()?;
                ensure!(
                    body.read_string(16)? == "RegistryCapture",
                    "unexpected capture login name"
                );
                let properties = body.read_varint()?;
                ensure!(
                    (0..=1024).contains(&properties),
                    "invalid login property count"
                );
                for _ in 0..properties {
                    body.read_string(32767)?;
                    body.read_string(32767)?;
                    if body.read_bool()? {
                        body.read_string(32767)?;
                    }
                }
                if self.version >= 776 {
                    body.read_uuid()?;
                }
                ensure!(
                    body.is_empty(),
                    "trailing login profile bytes for {}",
                    self.version
                );
                client.write_raw(self.acknowledged, &[]).await?;
                return Ok(());
            } else if frame.id == self.disconnect {
                anyhow::bail!("capture login refused: {}", body.read_string(32767)?);
            } else {
                anyhow::bail!("unexpected capture login packet {}", frame.id);
            }
        }
    }
}

#[cfg(test)]
#[path = "protocol_tests.rs"]
mod tests;
