use anyhow::{Result, anyhow};
use serde_json::json;

use super::Engine;
use crate::connect;

#[derive(Clone, Debug)]
pub enum Control {}

impl Engine {
    pub(super) async fn remote_control(&mut self, control: Control) -> Result<()> {
        match control {}
    }

    pub(super) async fn remote(&mut self, target: String, control: super::Control) -> Result<()> {
        let name = self
            .playback
            .as_ref()
            .and_then(|playback| playback.active_device_name.clone())
            .unwrap_or_else(|| "that device".into());
        let command = match control {
            super::Control::SetPlaying(true) => json!({"endpoint": "resume"}),
            super::Control::SetPlaying(false) => json!({"endpoint": "pause"}),
            super::Control::Next => json!({"endpoint": "skip_next"}),
            super::Control::Previous => json!({"endpoint": "skip_prev"}),
            super::Control::Seek(position) => json!({"endpoint": "seek_to", "value": position}),
            super::Control::Volume(volume) => {
                return connect::set_volume(&self.api, &self.device_id, &target, volume, None)
                    .await
                    .map_err(|_| anyhow!("Spotify didn’t accept that command for {name}."));
            }
            _ => return Ok(()),
        };
        self.send_command(&target, command)
            .await
            .map(|_| ())
            .map_err(|_| anyhow!("Spotify didn’t accept that command for {name}."))
    }
}
