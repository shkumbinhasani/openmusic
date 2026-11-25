use gpui::{AnyElement, Context, FontWeight, Pixels, div, prelude::*, px, rgb, rgba};

use super::{DOCK_HEIGHT, Palette, WINDOW_INSET};
use crate::{
    app::OpenMusic,
    areas::{account, audio, content, details, discovery, library, playback, playlists, system},
    overlay::{self, NowPlaying as NowPlayingBar},
};

pub const BAR_WEIGHT: u32 = 500;
const DOCK_GAP: f32 = 10.;
const BAR_ROOM: f32 = 280.;
const MIN_BAR: f32 = 150.;

pub struct DockItem {
    pub key: &'static str,
    pub weight: u32,
    pub width: f32,
    pub element: AnyElement,
    pub spare: bool,
}

impl DockItem {
    pub fn new(key: &'static str, weight: u32, width: f32, element: impl IntoElement) -> Self {
        Self {
            key,
            weight,
            width,
            element: element.into_any_element(),
            spare: false,
        }
    }

    pub fn spare(mut self) -> Self {
        self.spare = true;
        self
    }
}

pub type DockItems = fn(&OpenMusic, bool, Palette, &mut Context<OpenMusic>) -> Vec<DockItem>;

pub const DOCK_ITEMS: &[DockItems] = &[
    account::view::dock_items,
    audio::view::dock_items,
    content::view::dock_items,
    details::view::dock_items,
    discovery::view::dock_items,
    library::view::dock_items,
    playback::view::dock_items,
    playlists::view::dock_items,
    system::view::dock_items,
];

impl OpenMusic {
    pub fn dock_items(
        &self,
        roomy: bool,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Vec<DockItem> {
        let mut items: Vec<DockItem> = DOCK_ITEMS
            .iter()
            .flat_map(|items| items(self, roomy, palette, cx))
            .collect();
        items.sort_by_key(|item| item.weight);
        items
    }

    pub fn fitted_dock_items(
        &self,
        roomy: bool,
        dock_width: Pixels,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Vec<DockItem> {
        let mut items = self.dock_items(roomy, palette, cx);
        while crowded(f32::from(dock_width), &widths(&items)) {
            let Some(index) = items.iter().rposition(|item| item.spare) else {
                break;
            };
            items.remove(index);
        }
        items
    }

    pub fn dock_roomy(&self, dock_width: Pixels, palette: Palette, cx: &mut Context<Self>) -> bool {
        let widths = widths(&self.dock_items(true, palette, cx));
        bar_room(f32::from(dock_width), &widths) >= BAR_ROOM
    }

    fn now_playing_bar(&self, palette: Palette) -> impl IntoElement {
        let track = self.track();
        let title = track
            .map(|track| track.title.clone())
            .filter(|title| !title.is_empty())
            .unwrap_or_else(|| "Not playing".into());
        let subtitle = self
            .playback
            .notice
            .clone()
            .or_else(|| track.map(|track| track.artist.clone()))
            .unwrap_or_default();
        let state = NowPlayingBar {
            title: title.clone(),
            subtitle: subtitle.clone(),
            alert: self.playback.notice.is_some(),
            cover: track.and_then(|track| {
                let url = track.cover.clone()?;
                let image = self.covers.get(&url)?.clone();
                Some((url, image))
            }),
            position_ms: self.playback.now.position(),
            duration_ms: self.duration().unwrap_or(0),
            tooltip: self.stream_text(),
        };
        div().relative().flex_1().min_w_0().h_full().map(|bar| {
            if overlay::available() {
                bar.child(overlay::now_playing(state))
            } else {
                bar.flex()
                    .flex_col()
                    .justify_center()
                    .px_5()
                    .rounded_full()
                    .bg(rgba(palette.fill))
                    .child(
                        div()
                            .text_size(px(13.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .truncate()
                            .child(title),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(rgb(palette.muted))
                            .truncate()
                            .child(subtitle),
                    )
            }
        })
    }

    pub fn dock(
        &self,
        roomy: bool,
        dock_width: Pixels,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let slot = |item: DockItem| {
            div()
                .w(px(item.width))
                .h_full()
                .flex_shrink_0()
                .flex()
                .items_center()
                .justify_center()
                .child(item.element)
        };
        let (leading, trailing): (Vec<DockItem>, Vec<DockItem>) = self
            .fitted_dock_items(roomy, dock_width, palette, cx)
            .into_iter()
            .partition(|item| item.weight < BAR_WEIGHT);

        div()
            .id("dock")
            .occlude()
            .absolute()
            .left(px(WINDOW_INSET))
            .right(px(WINDOW_INSET))
            .bottom(px(WINDOW_INSET))
            .h(px(DOCK_HEIGHT))
            .flex()
            .items_center()
            .gap(px(DOCK_GAP))
            .children(leading.into_iter().map(slot))
            .child(self.now_playing_bar(palette))
            .children(trailing.into_iter().map(slot))
    }
}

fn widths(items: &[DockItem]) -> Vec<f32> {
    items.iter().map(|item| item.width).collect()
}

fn crowded(dock_width: f32, item_widths: &[f32]) -> bool {
    bar_room(dock_width, item_widths) < MIN_BAR
}

fn bar_room(dock_width: f32, item_widths: &[f32]) -> f32 {
    dock_width
        - item_widths
            .iter()
            .map(|width| width + DOCK_GAP)
            .sum::<f32>()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALWAYS: [f32; 7] = [36., 48., 60., 48., 36., 48., 48.];
    const WIDE_ONLY: [f32; 3] = [48., 48., 168.];

    fn roomy_at(window: f32) -> bool {
        let widths: Vec<f32> = ALWAYS.iter().chain(&WIDE_ONLY).copied().collect();
        bar_room(window - 248., &widths) >= BAR_ROOM
    }

    #[test]
    fn keeps_the_seek_bar_wide_before_adding_queue_devices_and_volume() {
        assert!(!roomy_at(800.));
        assert!(!roomy_at(1120.));
        assert!(roomy_at(1280.));
        assert!(roomy_at(1600.));
        assert!(bar_room(1120. - 248., &ALWAYS) > 400.);
    }

    #[test]
    fn drops_the_speed_button_before_the_bar_gets_too_narrow() {
        let with_speed: Vec<f32> = ALWAYS.iter().copied().chain([60.]).collect();
        assert!(!crowded(800. - 248., &ALWAYS));
        assert!(crowded(800. - 248., &with_speed));
        assert!(!crowded(900. - 248., &with_speed));
        assert!(!crowded(1120. - 248., &with_speed));
    }
}
