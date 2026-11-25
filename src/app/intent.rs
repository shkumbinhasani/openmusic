use gpui::{Context, Window};
use spotify_client::connect::{self, PlayOptions};

use crate::{
    app::{OpenMusic, Page},
    areas::playback::worker::Command,
};

#[derive(Clone, Debug, PartialEq)]
pub enum Intent {
    PlayPause,
    Play,
    Pause,
    Next,
    Previous,
    SeekTo(u32),
    SeekBy(i32),
    VolumeUp,
    VolumeDown,
    ToggleShuffle,
    CycleRepeat,
    ToggleLike,
    PlayContext(String),
    ShowMainWindow,
    ToggleMiniPlayer,
    Navigate(Page),
    Back,
    Forward,
    FocusSearch,
}

impl Intent {
    pub fn parse(text: &str) -> Option<Self> {
        let (name, rest) = text.split_once(' ').unwrap_or((text, ""));
        let rest = rest.trim();
        let bare = |intent: Self| rest.is_empty().then_some(intent);
        match name {
            "playpause" => bare(Self::PlayPause),
            "play" => bare(Self::Play),
            "pause" => bare(Self::Pause),
            "next" => bare(Self::Next),
            "previous" => bare(Self::Previous),
            "seekto" => rest.parse().ok().map(Self::SeekTo),
            "seekby" => rest.parse().ok().map(Self::SeekBy),
            "volumeup" => bare(Self::VolumeUp),
            "volumedown" => bare(Self::VolumeDown),
            "shuffle" => bare(Self::ToggleShuffle),
            "repeat" => bare(Self::CycleRepeat),
            "like" => bare(Self::ToggleLike),
            "playcontext" if !rest.is_empty() && !rest.contains(' ') => {
                Some(Self::PlayContext(rest.to_owned()))
            }
            "show" => bare(Self::ShowMainWindow),
            "mini" => bare(Self::ToggleMiniPlayer),
            "navigate" => Page::parse(rest).map(Self::Navigate),
            "back" => bare(Self::Back),
            "forward" => bare(Self::Forward),
            "focussearch" => bare(Self::FocusSearch),
            _ => None,
        }
    }
}

impl OpenMusic {
    pub fn perform(&mut self, intent: Intent, window: &mut Window, cx: &mut Context<Self>) {
        match intent {
            Intent::PlayPause | Intent::Play | Intent::Pause | Intent::Next | Intent::Previous
                if self.guest_locked() => {}
            Intent::PlayPause => self.toggle(cx),
            Intent::Play => self.set_playing(true, cx),
            Intent::Pause => self.set_playing(false, cx),
            Intent::Next => self.send(Command::Next),
            Intent::Previous => self.send(Command::Previous),
            Intent::SeekTo(position) => {
                if let Some(duration) = self.duration() {
                    self.seek_to(position.min(duration - 1), cx);
                }
            }
            Intent::SeekBy(delta) => {
                if let Some(duration) = self.duration() {
                    let position = i64::from(self.playback.now.position()) + i64::from(delta);
                    let position = position.clamp(0, i64::from(duration - 1)) as u32;
                    self.seek_to(position, cx);
                }
            }
            Intent::VolumeUp => self.step_volume(true, cx),
            Intent::VolumeDown => self.step_volume(false, cx),
            Intent::ToggleShuffle => self.toggle_shuffle(cx),
            Intent::CycleRepeat => self.cycle_repeat(cx),
            Intent::ToggleLike => self.like_current(),
            Intent::PlayContext(uri) => {
                if !self.needs_premium_here(false, cx) {
                    self.send(Command::PlayContext {
                        uri,
                        index: 0,
                        track_uri: None,
                        origin: connect::ORIGIN,
                        options: PlayOptions::default(),
                    });
                }
            }
            Intent::ShowMainWindow => self.show_main_window(window, cx),
            Intent::ToggleMiniPlayer => self.toggle_mini_player(window, cx),
            Intent::Navigate(page) => self.navigate(page),
            Intent::Back => self.back(),
            Intent::Forward => self.forward(),
            Intent::FocusSearch => self.focus_search(window, cx),
        }
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_debug_intents() {
        assert_eq!(Intent::parse("playpause"), Some(Intent::PlayPause));
        assert_eq!(Intent::parse("seekby -15000"), Some(Intent::SeekBy(-15000)));
        assert_eq!(Intent::parse("volumeup"), Some(Intent::VolumeUp));
        assert_eq!(Intent::parse("volumedown 4096"), None);
        assert_eq!(
            Intent::parse("playcontext spotify:album:1b5KESuOMQomF3wntoLAsk"),
            Some(Intent::PlayContext(
                "spotify:album:1b5KESuOMQomF3wntoLAsk".into()
            ))
        );
        assert_eq!(
            Intent::parse("navigate queue"),
            Some(Intent::Navigate(Page::Queue))
        );
        assert_eq!(Intent::parse("next now"), None);
        assert_eq!(Intent::parse("seekto soon"), None);
        assert_eq!(Intent::parse("playcontext"), None);
        assert_eq!(Intent::parse("dance"), None);
    }
}
