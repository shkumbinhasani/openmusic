use anyhow::{Result, anyhow};
use serde_json::json;

use super::{Control, Engine};
use crate::connect;

impl Engine {
    pub(super) async fn remote(&mut self, target: String, control: Control) -> Result<()> {
        let name = self
            .remote
            .as_ref()
            .and_then(|remote| remote.device_name.clone())
            .unwrap_or_else(|| "that device".into());
        let result = match control {
            Control::SetPlaying(true) => {
                connect::command(
                    &self.api,
                    &self.device_id,
                    &target,
                    json!({"endpoint": "resume"}),
                )
                .await
            }
            Control::SetPlaying(false) => {
                connect::command(
                    &self.api,
                    &self.device_id,
                    &target,
                    json!({"endpoint": "pause"}),
                )
                .await
            }
            Control::Next => {
                connect::command(
                    &self.api,
                    &self.device_id,
                    &target,
                    json!({"endpoint": "skip_next"}),
                )
                .await
            }
            Control::Previous => {
                connect::command(
                    &self.api,
                    &self.device_id,
                    &target,
                    json!({"endpoint": "skip_prev"}),
                )
                .await
            }
            Control::Seek(position) => {
                connect::command(
                    &self.api,
                    &self.device_id,
                    &target,
                    json!({"endpoint": "seek_to", "value": position}),
                )
                .await
            }
            Control::Volume(volume) => {
                connect::set_volume(&self.api, &self.device_id, &target, volume).await
            }
            _ => Ok(()),
        };
        result.map_err(|_| anyhow!("Spotify didn’t accept that command for {name}."))
    }
}
