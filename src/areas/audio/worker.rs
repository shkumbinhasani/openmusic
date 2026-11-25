use spotify_client::{
    catalog::LocalTrack,
    dealer::Message,
    engine::{
        Control,
        playback::{self, AudioReport, AudioStatus, SleepChoice, Update},
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
    PlayLocal {
        tracks: Vec<LocalTrack>,
        index: usize,
    },
    Speed {
        speed: f32,
        show: Option<String>,
    },
    Sleep(SleepChoice),
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
    Status(AudioStatus),
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
        Command::PlayLocal { tracks, index } => {
            ctx.handle
                .send(Control::Audio(playback::Control::PlayLocal {
                    tracks,
                    index,
                }))
        }
        Command::Sleep(choice) => ctx
            .handle
            .send(Control::Audio(playback::Control::Sleep(choice))),
        Command::Speed { speed, show } => {
            ctx.handle
                .send(Control::Audio(playback::Control::Speed(speed)));
            if let Some(show) = show {
                save_show_speed(ctx, show, speed);
            }
        }
    }
}

fn save_show_speed(ctx: &mut Ctx, show: String, speed: f32) {
    let api = ctx.api.clone();
    let device = descriptor(ctx);
    ctx.writes
        .push(Mutation::new(vec![format!("speed:{show}")], async move {
            let settings = vec![Setting::Speed(speed)];
            let event =
                match playback_settings::write_all(&api, &device, Some(&show), &settings).await {
                    Ok(()) => Event::Content {
                        content_id: show,
                        settings,
                    },
                    Err(_) => Event::WriteFailed {
                        message: "Couldn’t save this speed for the show. Try again.".into(),
                        remote: None,
                    },
                };
            Ok(event.into())
        }));
}

fn share_speeds(ctx: &Ctx, show: &str, settings: &[Setting]) {
    for setting in settings {
        if let Setting::Speed(speed) = setting {
            ctx.handle
                .send(Control::Audio(playback::Control::ShowSpeed {
                    show: show.to_owned(),
                    speed: *speed,
                }));
        }
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
        } => {
            share_speeds(ctx, &content_id, &settings);
            ctx.publish(Event::Content {
                content_id,
                settings,
            });
        }
        _ => sync(ctx),
    }
}

pub fn on_update(update: Update, ctx: &mut Ctx) {
    match update {
        Update::Report(report) => ctx.publish(Event::Report(report)),
        Update::Notice(message) => ctx.publish(crate::worker::Event::Notice(message)),
        Update::Status(status) => ctx.publish(Event::Status(status)),
    }
}

pub fn apply_settings(prefs: &AudioPrefs, ctx: &mut Ctx) {
    ctx.handle
        .send(Control::Audio(playback::Control::Settings(prefs.engine())));
}
