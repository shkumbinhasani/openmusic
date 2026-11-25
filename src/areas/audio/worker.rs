use spotify_client::{
    dealer::Message,
    engine::{
        Control,
        playback::{self, AudioReport, Update},
    },
    playback_settings::{self, Change, DeviceDescriptor, Remote, Setting},
};

use super::state::AudioPrefs;
use crate::worker::{Ctx, Mutation};

const SYNC: &str = "audio-settings";

pub enum Command {
    Sync,
    Write(Vec<Setting>),
    Report,
}

pub enum Event {
    Synced(Result<Remote, String>),
    Changed(Vec<Setting>),
    Content {
        content_id: String,
        settings: Vec<Setting>,
    },
    Written(Vec<Setting>),
    WriteFailed {
        message: String,
        remote: Option<Remote>,
    },
    Report(Box<AudioReport>),
}

impl Event {
    pub fn covers(&self) -> Vec<String> {
        Vec::new()
    }
}

fn device_name() -> String {
    std::env::var("OPENMUSIC_DEVICE_NAME").unwrap_or_else(|_| "OpenMusic".into())
}

fn descriptor(ctx: &Ctx) -> DeviceDescriptor {
    playback_settings::descriptor(&ctx.handle.device_id, &device_name())
}

fn sync(ctx: &mut Ctx) {
    let api = ctx.api.clone();
    let device = descriptor(ctx);
    ctx.tasks.spawn_keyed(SYNC, async move {
        let remote = playback_settings::read(&api, Some(&device), &[])
            .await
            .map_err(|error| error.to_string());
        Event::Synced(remote).into()
    });
}

pub fn handle(command: Command, ctx: &mut Ctx) {
    match command {
        Command::Sync => sync(ctx),
        Command::Write(settings) => {
            let api = ctx.api.clone();
            let device = descriptor(ctx);
            ctx.writes
                .push(Mutation::new(vec![SYNC.into()], async move {
                    let event = match playback_settings::write_all(&api, &device, None, &settings)
                        .await
                    {
                        Ok(()) => Event::Written(settings),
                        Err(_) => Event::WriteFailed {
                            message: "Couldn’t save this setting to Spotify. Try again.".into(),
                            remote: playback_settings::read(&api, Some(&device), &[]).await.ok(),
                        },
                    };
                    Ok(event.into())
                }));
        }
        Command::Report => ctx.handle.send(Control::Audio(playback::Control::Report)),
    }
}

pub fn on_push(message: &Message, ctx: &mut Ctx) {
    let Some(change) = playback_settings::change_from(message) else {
        return;
    };
    if !change.concerns_device(&ctx.handle.device_id) {
        return;
    }
    match change {
        Change::User {
            settings,
            complete: true,
        }
        | Change::Device {
            settings,
            complete: true,
            ..
        } => ctx.publish(Event::Changed(settings)),
        Change::Content {
            content_id,
            settings,
            complete: true,
        } => ctx.publish(Event::Content {
            content_id,
            settings,
        }),
        _ => sync(ctx),
    }
}

pub fn on_update(update: Update, ctx: &mut Ctx) {
    match update {
        Update::Report(report) => ctx.publish(Event::Report(report)),
    }
}

pub fn apply_settings(prefs: &AudioPrefs, ctx: &mut Ctx) {
    ctx.handle
        .send(Control::Audio(playback::Control::Settings(prefs.engine())));
}
