//! Status state handler — the vanilla "server list ping" choreography.
//!
//! Packet IDs and layouts come only from `mc-protocol`.

use mc_protocol::packets::status::{PingRequest, PongResponse, StatusRequest, StatusResponse};
use mc_protocol::{PROTOCOL_VERSION, TARGET_RELEASE};

use crate::connection::Connection;

/// Builds the JSON payload the vanilla client renders in its server list.
/// Advertise the selected available endpoint, or canonical for unknown ids.
#[must_use]
pub fn build_status_json(
    motd: &str,
    max_players: u32,
    online: usize,
    version: Option<mc_data::protocol_versions::Version>,
) -> String {
    let (name, protocol) = version.map_or((TARGET_RELEASE, PROTOCOL_VERSION), |version| {
        (version.release, version.protocol)
    });
    serde_json::json!({
        "version": {
            "name": name,
            "protocol": protocol,
        },
        "players": {
            "max": max_players,
            "online": online,
            "sample": [],
        },
        "description": { "text": motd },
        "enforcesSecureChat": false,
    })
    .to_string()
}

/// Vanilla 1.7+ pattern, uncompressed: StatusRequest -> StatusResponse ->
/// PingRequest -> PongResponse (echo). Caller closes TCP afterwards.
pub async fn handle_status(
    conn: &mut Connection,
    motd: &str,
    max_players: u32,
    online: usize,
    version: Option<mc_data::protocol_versions::Version>,
) -> anyhow::Result<()> {
    let _ = conn
        .read_packet::<StatusRequest>(mc_protocol::State::Status)
        .await?;
    conn.send_packet(&StatusResponse {
        json: build_status_json(motd, max_players, online, version),
    })
    .await?;
    let ping = conn
        .read_packet::<PingRequest>(mc_protocol::State::Status)
        .await?;
    conn.send_packet(&PongResponse {
        payload: ping.payload,
    })
    .await?;
    Ok(())
}

#[cfg(test)]
#[path = "status_tests.rs"]
mod tests;
