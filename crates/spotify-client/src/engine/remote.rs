use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};
use spotify_proto::connectstate::ProvidedTrack;
use tokio::sync::watch;

use super::Engine;
use crate::connect::{
    self, Allowed, CommandError, ConnectDevice, PlayOptions, Playback, QueueKey, Repeat, Restore,
    Shuffle,
};

const QUEUE_EDIT_WINDOW: Duration = Duration::from_secs(3);

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
}

impl State {
    pub(super) fn redirect(&self, target: &str) -> String {
        target.to_owned()
    }

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
                if !self.playing_model()?.allowed.play_next {
                    bail!("You can’t add songs to the queue right now.");
                }
                self.player_command(connect::play_next_command(&uris))
                    .await
                    .map_err(|_| anyhow!("Couldn’t add that to the queue."))
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
                    .map_err(|_| anyhow!("Spotify didn’t start playback."))
            }
            Control::RefreshCluster => self
                .refresh_cluster()
                .await
                .map_err(|_| anyhow!("Couldn’t reach Spotify Connect.")),
            Control::Transfer(id) => self.transfer_to(id).await,
            Control::DeviceVolume { device, volume } => self.device_volume(device, volume).await,
        }
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
        if device.active || (here && nothing_active) {
            return Ok(());
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
        if !self.playing_model()?.allowed.add_to_queue {
            bail!("You can’t add songs to the queue right now.");
        }
        if let [uri] = uris.as_slice() {
            return self
                .player_command(connect::add_to_queue_command(uri))
                .await
                .map_err(|_| anyhow!("Couldn’t add that to the queue."));
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
            .map_err(|_| anyhow!("Couldn’t skip to that song."))
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
                    return Ok(());
                };
                match connect::previous_command(self.remote_position(), &playback.allowed) {
                    Some(command) => command,
                    None => bail!("You can’t go back right now."),
                }
            }
            super::Control::Seek(position) => json!({"endpoint": "seek_to", "value": position}),
            super::Control::Volume(volume) => {
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
            .map_err(|_| anyhow!("Spotify didn’t accept that command for {name}."))
    }
}
