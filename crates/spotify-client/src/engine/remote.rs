use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};
use spotify_proto::connectstate::ProvidedTrack;
use tokio::sync::watch;

use super::Engine;
use crate::{
    connect::{
        self, Allowed, CommandError, ConnectDevice, PlayOptions, Playback, QueueKey, Repeat,
        Restore, Shuffle,
    },
    dj, jam,
};

const QUEUE_EDIT_WINDOW: Duration = Duration::from_secs(3);
pub const JAM_ENDED: &str = "The Jam has ended.";
const JAM_TRANSFER: &str = "Leave the Jam to play on another device.";
const JAM_VOLUME: &str = "Only the host can change the speaker volume.";

#[derive(Debug)]
pub struct JamGone;

impl std::fmt::Display for JamGone {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(JAM_ENDED)
    }
}

impl std::error::Error for JamGone {}

fn keep_jam(error: anyhow::Error, fallback: impl Into<String>) -> anyhow::Error {
    if error.downcast_ref::<JamGone>().is_some() {
        error
    } else {
        anyhow!(fallback.into())
    }
}

#[derive(Clone, Debug)]
pub enum Control {
    SetShuffle(Shuffle),
    SetRepeat(Repeat),
    AddToQueue(Vec<String>),
    PlayNext(Vec<String>),
    RemoveFromQueue(Vec<QueueKey>),
    MoveInQueue {
        moved: Vec<QueueKey>,
        anchor: QueueKey,
        after: bool,
    },
    ClearQueue,
    JumpTo(QueueKey),
    ShufflePlay {
        uri: String,
        origin: &'static str,
        options: PlayOptions,
    },
    RefreshCluster,
    Transfer(String),
    DeviceVolume {
        device: String,
        volume: u16,
    },
    Jam(Option<String>),
    PlayDj {
        context_url: String,
    },
    DjJump,
}

#[derive(Debug)]
struct VolumeSender {
    target: String,
    sender: watch::Sender<u16>,
}

#[derive(Debug, Default)]
pub(super) struct State {
    queue_edit: Option<(String, Instant)>,
    volume: Option<VolumeSender>,
    jam: Option<String>,
}

impl State {
    fn queue_busy(&self, revision: &str) -> bool {
        self.queue_edit
            .as_ref()
            .is_some_and(|(edited, at)| edited == revision && at.elapsed() < QUEUE_EDIT_WINDOW)
    }
}

enum Edit {
    Remove(Vec<QueueKey>),
    Move {
        moved: Vec<QueueKey>,
        anchor: QueueKey,
        after: bool,
    },
    Clear,
    Add(Vec<String>),
}

impl Edit {
    fn permitted(&self, allowed: &Allowed) -> bool {
        allowed.set_queue
            && match self {
                Self::Remove(_) | Self::Clear => allowed.remove_from_queue,
                Self::Move { .. } => allowed.reorder_queue,
                Self::Add(_) => allowed.add_to_queue,
            }
    }

    fn apply(&self, next: &[ProvidedTrack]) -> Vec<ProvidedTrack> {
        match self {
            Self::Remove(keys) => connect::queue_without(next, keys),
            Self::Move {
                moved,
                anchor,
                after,
            } => connect::queue_moved(next, moved, anchor, *after),
            Self::Clear => connect::queue_cleared(next),
            Self::Add(uris) => connect::queue_with_added(next, uris),
        }
    }
}

fn gone(error: &anyhow::Error) -> bool {
    matches!(
        error.downcast_ref::<CommandError>(),
        Some(CommandError::DeviceGone)
    )
}

fn refused(error: &anyhow::Error) -> bool {
    matches!(
        error.downcast_ref::<CommandError>(),
        Some(CommandError::Refused(_))
    )
}

fn device_name(device: &ConnectDevice) -> String {
    if device.this_mac {
        "This Mac".to_owned()
    } else {
        device.name.clone()
    }
}

fn blocked_notice(name: &str, reason: &str) -> String {
    match reason {
        "Unavailable" => format!("{name} isn’t available right now."),
        "Can’t play this" => format!("{name} can’t play this."),
        _ => "You can’t move playback right now.".to_owned(),
    }
}

impl Engine {
    pub(super) async fn routed_command(
        &mut self,
        target: &str,
        command: Value,
    ) -> Result<Option<String>> {
        if let Some(session) = self.remote.jam.clone() {
            if jam::redirect(&self.api, &session, &self.device_id, command).await? {
                return Ok(None);
            }
            self.remote.jam = None;
            bail!(JamGone);
        }
        let (to, alias) = connect::device_target(target);
        connect::command(&self.api, &self.device_id, to, command, alias).await
    }

    pub(super) fn control_target(&self, control: &super::Control) -> Option<String> {
        if let Some(target) = self.remote_target() {
            return Some(target);
        }
        let player = matches!(
            control,
            super::Control::SetPlaying(_)
                | super::Control::Next
                | super::Control::Previous
                | super::Control::Seek(_)
        );
        (self.redirected() && player).then(|| self.command_target())
    }

    fn redirected(&self) -> bool {
        self.remote.jam.is_some()
    }

    fn command_target(&self) -> String {
        self.playback
            .as_ref()
            .and_then(|playback| {
                playback
                    .active_device()
                    .map(|device| device.id.clone())
                    .or_else(|| playback.active_device_id.clone())
            })
            .unwrap_or_else(|| self.device_id.clone())
    }

    fn uses_options(&self) -> bool {
        self.playback
            .as_ref()
            .and_then(Playback::active_device)
            .is_none_or(|device| device.supports_set_options)
    }

    fn playing_model(&self) -> Result<&Playback> {
        self.playback
            .as_ref()
            .filter(|playback| playback.track.is_some())
            .ok_or_else(|| anyhow!("Nothing is playing."))
    }

    async fn refresh_cluster(&mut self) -> Result<()> {
        let cluster = connect::fetch_cluster(&self.api).await?;
        self.playback = Some(connect::playback(&cluster, &self.device_id));
        Ok(())
    }

    async fn player_command(&mut self, command: Value) -> Result<()> {
        let target = self.command_target();
        let result = self.send_command(&target, command).await.map(|_| ());
        if let Err(error) = &result
            && gone(error)
        {
            let _ = self.refresh_cluster().await;
        }
        result
    }

    pub(super) async fn remote_control(&mut self, control: Control) -> Result<()> {
        match control {
            Control::SetShuffle(mode) => self.set_shuffle(mode).await,
            Control::SetRepeat(mode) => self.set_repeat(mode).await,
            Control::AddToQueue(uris) => self.add_to_queue(uris).await,
            Control::PlayNext(uris) => {
                if uris.is_empty() {
                    return Ok(());
                }
                if !self.redirected() && !self.playing_model()?.allowed.play_next {
                    bail!("You can’t add songs to the queue right now.");
                }
                self.player_command(connect::play_next_command(&uris))
                    .await
                    .map_err(|error| keep_jam(error, "Couldn’t add that to the queue."))
            }
            Control::RemoveFromQueue(keys) => self.edit_queue(Edit::Remove(keys)).await,
            Control::MoveInQueue {
                moved,
                anchor,
                after,
            } => {
                self.edit_queue(Edit::Move {
                    moved,
                    anchor,
                    after,
                })
                .await
            }
            Control::ClearQueue => self.edit_queue(Edit::Clear).await,
            Control::JumpTo(key) => self.jump_to(key).await,
            Control::ShufflePlay {
                uri,
                origin,
                options,
            } => {
                let own = self.device_id.clone();
                self.send_command(&own, connect::shuffle_play_command(&uri, origin, &options))
                    .await
                    .map(|_| ())
                    .map_err(|error| keep_jam(error, "Spotify didn’t start playback."))
            }
            Control::RefreshCluster => self
                .refresh_cluster()
                .await
                .map_err(|_| anyhow!("Couldn’t reach Spotify Connect.")),
            Control::Transfer(id) => self.transfer_to(id).await,
            Control::DeviceVolume { device, volume } => self.device_volume(device, volume).await,
            Control::Jam(session) => {
                self.remote.jam = session;
                Ok(())
            }
            Control::PlayDj { context_url } => {
                let target = self.command_target();
                self.send_command(&target, dj::play_command(&context_url))
                    .await
                    .map(|_| ())
                    .map_err(|error| keep_jam(error, "Spotify didn’t start the DJ."))
            }
            Control::DjJump => self.dj_jump().await,
        }
    }

    async fn dj_jump(&mut self) -> Result<()> {
        let command = match self.playing_model().ok().and_then(dj::jump) {
            Some(dj::Jump::Signal) => dj::signal_command(dj::JUMP),
            Some(dj::Jump::SkipTo(track)) => connect::skip_to_command(&track),
            None => bail!("The DJ can’t switch right now."),
        };
        self.player_command(command)
            .await
            .map_err(|error| keep_jam(error, "The DJ can’t switch right now."))
    }

    fn device(&self, id: &str) -> Option<ConnectDevice> {
        self.playback
            .as_ref()?
            .devices
            .iter()
            .find(|device| device.id == id)
            .cloned()
    }

    async fn transfer_to(&mut self, id: String) -> Result<()> {
        if self.redirected() {
            bail!(JAM_TRANSFER);
        }
        let own = self.device_id.clone();
        let (to, alias) = connect::device_target(&id);
        let here = to == own;
        let Some(device) = self.device(&id) else {
            let _ = self.refresh_cluster().await;
            bail!("That device is no longer available.");
        };
        let name = device_name(&device);
        let playback = self.playback.as_ref();
        let nothing_active = playback.is_none_or(|playback| playback.active_device_id.is_none());
        if device.active {
            return Ok(());
        }
        if here && nothing_active {
            return self
                .revive_here(Restore::Keep)
                .await
                .map(|_| ())
                .map_err(|_| anyhow!("Couldn’t connect to {name}."));
        }
        if let Some(reason) = &device.blocked {
            bail!(blocked_notice(&name, reason));
        }
        if playback.is_some_and(|playback| playback.track.is_some() && !playback.allowed.transfer) {
            bail!("You can’t move playback right now.");
        }
        match connect::transfer(&self.api, &own, to, Restore::Keep, alias).await {
            Ok(()) => Ok(()),
            Err(error) => {
                if gone(&error) || refused(&error) {
                    let _ = self.refresh_cluster().await;
                }
                Err(anyhow!("Couldn’t connect to {name}."))
            }
        }
    }

    pub(super) async fn revive_here(&mut self, restore: Restore) -> Result<bool> {
        if self.redirected() || !self.playback.as_ref().is_some_and(Playback::orphaned) {
            return Ok(false);
        }
        let own = self.device_id.clone();
        connect::transfer(&self.api, &own, &own, restore, None).await?;
        Ok(true)
    }

    async fn device_volume(&mut self, id: String, volume: u16) -> Result<()> {
        let (device, _) = connect::device_target(&id);
        if device == self.device_id {
            if self.remote_target().is_some() {
                bail!("This Mac isn’t playing right now.");
            }
            return Box::pin(self.control(super::Control::Volume(volume))).await;
        }
        let Some(entry) = self.device(&id) else {
            bail!("That device is no longer available.");
        };
        if entry.volume.is_none() {
            bail!("You can’t change the volume on {}.", entry.name);
        }
        self.queue_volume(id, volume);
        Ok(())
    }

    fn queue_volume(&mut self, target: String, volume: u16) {
        if let Some(queued) = self
            .remote
            .volume
            .as_ref()
            .filter(|queued| queued.target == target && !queued.sender.is_closed())
        {
            queued.sender.send_replace(volume);
            return;
        }
        let (sender, mut receiver) = watch::channel(volume);
        receiver.mark_changed();
        let api = self.api.clone();
        let from = self.device_id.clone();
        let to = target.clone();
        tokio::spawn(async move {
            while receiver.changed().await.is_ok() {
                let volume = *receiver.borrow_and_update();
                let (device, alias) = connect::device_target(&to);
                let _ = connect::set_volume(&api, &from, device, volume, alias).await;
            }
        });
        self.remote.volume = Some(VolumeSender { target, sender });
    }

    fn remote_position(&self) -> u32 {
        let Some(playback) = self.playback.as_ref() else {
            return 0;
        };
        let track = playback.track.as_ref().map(|track| &track.uri);
        match &self.published {
            Some((key, position, at)) if !key.local && key.track.as_ref() == track => {
                let elapsed = if key.playing {
                    at.elapsed().as_millis().min(u128::from(u32::MAX)) as u32
                } else {
                    0
                };
                position.saturating_add(elapsed)
            }
            _ => playback.position_ms,
        }
    }

    async fn set_shuffle(&mut self, mode: Shuffle) -> Result<()> {
        let allowed = self.playing_model()?.allowed;
        if !allowed.shuffle {
            bail!("Shuffle isn’t available for this.");
        }
        if mode == Shuffle::Smart && !allowed.smart_shuffle {
            bail!("Smart Shuffle isn’t available for this.");
        }
        let failed = || anyhow!("Couldn’t change shuffle.");
        if !self.uses_options() {
            if mode == Shuffle::Smart {
                bail!("Smart Shuffle isn’t available on this device.");
            }
            return self
                .player_command(connect::shuffle_command(mode, false))
                .await
                .map_err(|_| failed());
        }
        match self
            .player_command(connect::shuffle_command(mode, true))
            .await
        {
            Ok(()) => Ok(()),
            Err(error) if mode != Shuffle::Smart && !gone(&error) && !refused(&error) => self
                .player_command(connect::shuffle_command(mode, false))
                .await
                .map_err(|_| failed()),
            Err(_) => Err(failed()),
        }
    }

    async fn set_repeat(&mut self, mode: Repeat) -> Result<()> {
        let allowed = self.playing_model()?.allowed;
        let possible = match mode {
            Repeat::Off => allowed.repeat_context || allowed.repeat_track,
            Repeat::Context => allowed.repeat_context,
            Repeat::Track => allowed.repeat_track,
        };
        if !possible {
            bail!("Repeat isn’t available for this.");
        }
        let failed = || anyhow!("Couldn’t change repeat.");
        let use_options = self.uses_options();
        let mut sent = Ok(());
        for command in connect::repeat_commands(mode, &allowed, use_options) {
            sent = self.player_command(command).await;
            if sent.is_err() {
                break;
            }
        }
        match sent {
            Ok(()) => Ok(()),
            Err(error) if use_options && !gone(&error) && !refused(&error) => {
                for command in connect::repeat_commands(mode, &allowed, false) {
                    self.player_command(command).await.map_err(|_| failed())?;
                }
                Ok(())
            }
            Err(_) => Err(failed()),
        }
    }

    async fn add_to_queue(&mut self, uris: Vec<String>) -> Result<()> {
        if uris.is_empty() {
            return Ok(());
        }
        if self.redirected() {
            let (first, rest) = uris.split_first().expect("uris is not empty");
            self.player_command(connect::add_to_queue_command(first))
                .await
                .map_err(|error| keep_jam(error, "Couldn’t add that to the Jam’s queue."))?;
            if let (Some(session), false) = (self.remote.jam.clone(), rest.is_empty()) {
                let (api, from, rest) = (self.api.clone(), self.device_id.clone(), rest.to_vec());
                tokio::spawn(async move {
                    for uri in rest {
                        let command = connect::add_to_queue_command(&uri);
                        if !matches!(
                            jam::redirect(&api, &session, &from, command).await,
                            Ok(true)
                        ) {
                            break;
                        }
                    }
                });
            }
            return Ok(());
        }
        if !self.playing_model()?.allowed.add_to_queue {
            bail!("You can’t add songs to the queue right now.");
        }
        if let [uri] = uris.as_slice() {
            return self
                .player_command(connect::add_to_queue_command(uri))
                .await
                .map_err(|error| keep_jam(error, "Couldn’t add that to the queue."));
        }
        self.edit_queue(Edit::Add(uris)).await
    }

    async fn edit_queue(&mut self, edit: Edit) -> Result<()> {
        let playback = self.playing_model()?;
        if !edit.permitted(&playback.allowed) {
            bail!("You can’t change the queue right now.");
        }
        let revision = playback.queue.revision.clone();
        if self.remote.queue_busy(&revision) {
            bail!("The queue is still updating. Try again in a moment.");
        }
        let next = edit.apply(&playback.queue.raw_next);
        if next == playback.queue.raw_next {
            return Ok(());
        }
        let command = connect::set_queue_command(&next, &playback.queue.raw_prev, &revision);
        match self.player_command(command).await {
            Ok(()) => {
                self.remote.queue_edit = Some((revision, Instant::now()));
                Ok(())
            }
            Err(_) => {
                self.remote.queue_edit = None;
                let _ = self.refresh_cluster().await;
                Err(anyhow!("Couldn’t change the queue. Try again."))
            }
        }
    }

    async fn jump_to(&mut self, key: QueueKey) -> Result<()> {
        let playback = self.playing_model()?;
        if !playback.allowed.jump_in_queue {
            bail!("You can’t skip to that song right now.");
        }
        let Some(item) = playback
            .queue
            .raw_next
            .iter()
            .find(|track| QueueKey::of(track) == key)
            .or_else(|| {
                playback
                    .queue
                    .raw_next
                    .iter()
                    .find(|track| key.uid.is_empty() && track.uri == key.uri)
            })
            .cloned()
        else {
            bail!("That song is no longer in the queue.");
        };
        self.player_command(connect::skip_to_command(&item))
            .await
            .map_err(|error| keep_jam(error, "Couldn’t skip to that song."))
    }

    pub(super) async fn remote(&mut self, _target: String, control: super::Control) -> Result<()> {
        let target = self.command_target();
        let active = self.playback.as_ref().and_then(Playback::active_device);
        let name = active
            .map(|device| device.name.clone())
            .or_else(|| {
                self.playback
                    .as_ref()
                    .and_then(|playback| playback.active_device_name.clone())
            })
            .unwrap_or_else(|| "that device".into());
        let command = match control {
            super::Control::SetPlaying(true) => json!({"endpoint": "resume"}),
            super::Control::SetPlaying(false) => json!({"endpoint": "pause"}),
            super::Control::Next => json!({"endpoint": "skip_next"}),
            super::Control::Previous => {
                let Some(playback) = self
                    .playback
                    .as_ref()
                    .filter(|playback| playback.track.is_some())
                else {
                    if !self.redirected() {
                        return Ok(());
                    }
                    return self
                        .send_command(&target, json!({"endpoint": "skip_prev"}))
                        .await
                        .map(|_| ())
                        .map_err(|error| keep_jam(error, "Couldn’t go back in the Jam."));
                };
                match connect::previous_command(self.remote_position(), &playback.allowed) {
                    Some(command) => command,
                    None => bail!("You can’t go back right now."),
                }
            }
            super::Control::Seek(position) => json!({"endpoint": "seek_to", "value": position}),
            super::Control::Volume(volume) => {
                if active.is_none() && self.redirected() {
                    bail!(JAM_VOLUME);
                }
                if active.is_some_and(|device| device.volume.is_none()) {
                    bail!("You can’t change the volume on {name}.");
                }
                self.queue_volume(target, volume);
                return Ok(());
            }
            _ => return Ok(()),
        };
        self.send_command(&target, command)
            .await
            .map(|_| ())
            .map_err(|error| {
                keep_jam(
                    error,
                    format!("Spotify didn’t accept that command for {name}."),
                )
            })
    }
}
