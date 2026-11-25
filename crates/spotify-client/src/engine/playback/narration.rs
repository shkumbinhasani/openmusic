use anyhow::{Result, bail};
use serde_json::Value;

use super::{Playing, Update};
use crate::engine::Engine;

const MAX_RUN: u32 = 8;
const VOICE: &str = "OpenMusic can’t play the DJ’s voice, so it skips to the next song.";
const ONLY_VOICE: &str = "The DJ sent only voice clips, and OpenMusic can’t play them.";

fn flagged(value: &Value) -> bool {
    value.as_bool().unwrap_or(false)
        || value
            .as_str()
            .is_some_and(|text| text.eq_ignore_ascii_case("true"))
}

fn named(value: &Value) -> bool {
    value
        .as_str()
        .is_some_and(|kind| kind.eq_ignore_ascii_case("narration"))
}

pub(super) fn is_narration(track: &Value) -> bool {
    let metadata = &track["metadata"];
    let uri = |value: &Value| {
        value
            .as_str()
            .is_some_and(|uri| uri.starts_with("spotify:media:"))
    };
    uri(&metadata["uri"])
        || uri(&track["uri"])
        || flagged(&metadata["is_narration"])
        || flagged(&track["is_narration"])
        || named(&metadata["content_type"])
        || named(&track["content_type"])
        || named(&metadata["media_type"])
}

impl Engine {
    pub(super) async fn skip_narration(&mut self, playing: Playing) -> Result<()> {
        self.audio.narration_run += 1;
        if self.audio.narration_run > MAX_RUN {
            self.audio.narration_run = 0;
            bail!(ONLY_VOICE);
        }
        if !self.audio.narration_told {
            self.audio.narration_told = true;
            self.audio.updates.push(Update::Notice(VOICE.into()));
        }
        Box::pin(self.move_past(playing, "advance", "narration skipped")).await
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn spots_dj_voice_clips() {
        assert!(is_narration(
            &json!({"metadata": {"uri": "spotify:media:0000000000000000000001"}})
        ));
        assert!(is_narration(
            &json!({"uri": "spotify:media:0000000000000000000001", "metadata": {}})
        ));
        assert!(is_narration(&json!({"metadata": {
            "uri": "spotify:track:0000000000000000000002", "is_narration": "true"}})));
        assert!(is_narration(&json!({"metadata": {
            "uri": "spotify:track:0000000000000000000002", "is_narration": true}})));
        assert!(is_narration(&json!({"metadata": {
            "uri": "spotify:track:0000000000000000000002", "content_type": "NARRATION"}})));
        assert!(!is_narration(&json!({"metadata": {
            "uri": "spotify:track:0000000000000000000002", "is_narration": "false"}})));
        assert!(!is_narration(
            &json!({"metadata": {"uri": "spotify:episode:0000000000000000000003"}})
        ));
        assert!(!is_narration(&json!({})));
    }
}
