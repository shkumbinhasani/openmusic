use std::sync::Mutex;

use gpui::{AnyElement, Context, prelude::*, px};
use spotify_client::{
    Track,
    playback_settings::{MAX_SPEED, MIN_SPEED, round_speed},
};

use super::state::{SPEED_KEY, speed_text};
use crate::{
    app::OpenMusic,
    overlay::{self, ButtonSpec, Label},
    ui::{
        Palette,
        dock::DockItem,
        menu::{Group, MenuItem, Row, Surface, Target},
        sidebar::NavEntry,
        tracks::RowOptions,
    },
};

const SPEED_WEIGHT: u32 = 620;

fn speed_label(speed: f32) -> &'static str {
    static LABELS: Mutex<Vec<(u32, &'static str)>> = Mutex::new(Vec::new());
    let speed = round_speed(speed.clamp(MIN_SPEED, MAX_SPEED));
    let key = (speed * 100.).round() as u32;
    let Ok(mut labels) = LABELS.lock() else {
        return "1×";
    };
    if let Some((_, label)) = labels.iter().find(|(known, _)| *known == key) {
        return label;
    }
    let label: &'static str = Box::leak(speed_text(speed).into_boxed_str());
    labels.push((key, label));
    label
}

impl OpenMusic {
    pub fn playback_section(
        &self,
        _palette: Palette,
        _cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        None
    }

    pub fn audio_quality_section(
        &self,
        _palette: Palette,
        _cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        None
    }

    pub fn volume_section(&self, _palette: Palette, _cx: &mut Context<Self>) -> Option<AnyElement> {
        None
    }

    pub fn device_section(&self, _palette: Palette, _cx: &mut Context<Self>) -> Option<AnyElement> {
        None
    }

    fn speed_control(&self, palette: Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let speed = self.current_speed();
        let enabled = self.can_set_speed();
        let spec = ButtonSpec {
            label: Label::Text(speed_label(speed)),
            prominent: (speed - 1.).abs() > 0.01,
            enabled,
            tooltip: Some(if enabled {
                "Playback speed".to_owned()
            } else {
                "Speed can’t change for this episode".to_owned()
            }),
        };
        self.glass_control(SPEED_KEY, spec, true, palette, cx)
            .h(px(36.))
            .w(px(56.))
            .child(overlay::anchor(SPEED_KEY))
    }
}

pub fn menu_items(target: &Target, app: &OpenMusic) -> Vec<(Group, MenuItem)> {
    let Target::Track(track) = target else {
        return Vec::new();
    };
    let current = app.playback.now.track.as_ref();
    if track.surface != Surface::NowPlaying
        || current.is_none_or(|current| current.uri != track.track.uri)
        || !app.playback.now.episode
    {
        return Vec::new();
    }
    vec![(
        Group::Queue,
        MenuItem::submenu(
            format!("Playback speed ({})", speed_text(app.current_speed())),
            app.speed_items(),
        )
        .symbol("speedometer"),
    )]
}

pub fn nav_entries(_app: &OpenMusic) -> Vec<NavEntry> {
    Vec::new()
}

pub fn dock_items(
    app: &OpenMusic,
    _roomy: bool,
    palette: Palette,
    cx: &mut Context<OpenMusic>,
) -> Vec<DockItem> {
    let mut items = Vec::new();
    if app.playback.now.episode && app.playback.now.track.is_some() {
        items.push(DockItem::new(
            SPEED_KEY,
            SPEED_WEIGHT,
            60.,
            app.speed_control(palette, cx),
        ));
    }
    items
}

pub fn row_accessories(
    _app: &OpenMusic,
    _track: &Track,
    _row: Row,
    _options: &RowOptions,
    _palette: Palette,
    _cx: &mut Context<OpenMusic>,
) -> Vec<AnyElement> {
    Vec::new()
}

pub fn row_dimmed(_app: &OpenMusic, _track: &Track, _row: Row, _options: &RowOptions) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reuses_one_label_per_speed() {
        let first = speed_label(1.5);
        assert_eq!(first, "1.5×");
        assert!(std::ptr::eq(first, speed_label(1.49)));
        assert_eq!(speed_label(9.), "3.5×");
        assert_eq!(speed_label(0.), "0.5×");
    }
}
