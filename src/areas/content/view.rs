use gpui::{AnyElement, Context, FontWeight, MouseButton, Pixels, div, prelude::*, px, rgb, rgba};
use spotify_client::Track;

use crate::{
    app::{OpenMusic, Page},
    icons,
    overlay::Label,
    ui::{
        Palette,
        components::bone,
        dock::DockItem,
        menu::{Group, MenuItem, Row, Surface, Target},
        sidebar::NavEntry,
        tracks::RowOptions,
    },
};

impl OpenMusic {
    pub fn display_section(
        &self,
        _palette: Palette,
        _cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        None
    }

    pub fn lyrics_page(&self, palette: Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let track = self.track();
        let lyrics = self.current_lyrics();
        let synced = lyrics.is_some_and(|lyrics| lyrics.synced);
        let lines = lyrics
            .map(|lyrics| lyrics.lines.as_slice())
            .unwrap_or_default();
        let active = self.content.lyrics_line;
        let status = match (track, lyrics) {
            (None, _) => Some("Play a song to see its lyrics."),
            (Some(_), None) if self.content.lyrics_loading => None,
            (Some(_), None) => Some("Lyrics aren’t available for this song."),
            (Some(_), Some(_)) => None,
        };
        let heading = track
            .map(|track| {
                if track.artist.is_empty() {
                    track.title.clone()
                } else {
                    format!("{} · {}", track.title, track.artist)
                }
            })
            .unwrap_or_default();

        div()
            .id("lyrics")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.content.lyrics_scroll)
            .px(px(48.))
            .pt(px(40.))
            .pb(px(320.))
            .child(
                div()
                    .pb_8()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_size(px(30.))
                            .font_weight(FontWeight::BOLD)
                            .child("Lyrics"),
                    )
                    .child(
                        div()
                            .text_color(rgb(palette.muted))
                            .truncate()
                            .child(heading)
                            .on_mouse_down(
                                MouseButton::Right,
                                self.on_menu(
                                    None,
                                    |app| app.now_playing_target(Surface::Header),
                                    cx,
                                ),
                            ),
                    )
                    .when_some(status, |header, status| {
                        header.child(
                            div()
                                .pt_6()
                                .text_size(px(17.))
                                .text_color(rgb(palette.muted))
                                .child(status),
                        )
                    }),
            )
            .when(
                self.content.lyrics_loading && lines.is_empty() && track.is_some(),
                |page| {
                    page.child(
                        div().flex().flex_col().gap(px(22.)).px_3().children(
                            [0.62, 0.48, 0.7, 0.4, 0.56, 0.66, 0.36, 0.52]
                                .into_iter()
                                .enumerate()
                                .map(|(index, width)| {
                                    bone("lyric-line", index, palette)
                                        .w(gpui::relative(width * 0.85))
                                        .max_w(px(640.))
                                        .h(px(24.))
                                        .rounded(px(6.))
                                }),
                        ),
                    )
                },
            )
            .children(lines.iter().enumerate().map(|(index, line)| {
                let current = active == Some(index);
                let start = line.start_ms;
                let pause = line.words.is_empty();
                let (size, height) = match (pause, current) {
                    (true, _) => (18., 26.),
                    (false, true) => (30., 38.),
                    (false, false) => (26., 34.),
                };
                div()
                    .id(("lyric", index))
                    .max_w(px(760.))
                    .px_3()
                    .py(px(if pause { 4. } else { 8. }))
                    .rounded(px(12.))
                    .text_size(px(size))
                    .line_height(px(height))
                    .font_weight(FontWeight::BOLD)
                    .text_color(rgb(if current || !synced {
                        palette.text
                    } else {
                        palette.muted
                    }))
                    .when(synced && !current, |line| line.opacity(0.55))
                    .when(synced, |line| {
                        line.cursor_pointer()
                            .hover(move |line| line.bg(rgba(palette.press)).opacity(1.))
                            .on_click(cx.listener(move |this, _, _, cx| this.seek_to(start, cx)))
                    })
                    .child(if pause {
                        "• • •".to_owned()
                    } else {
                        line.words.clone()
                    })
            }))
            .when_some(
                lyrics.filter(|lyrics| !lyrics.provider.is_empty()),
                |page, lyrics| {
                    page.child(
                        div()
                            .px_3()
                            .pt_8()
                            .text_size(px(12.))
                            .text_color(rgb(palette.muted))
                            .child(format!("Lyrics provided by {}", lyrics.provider)),
                    )
                },
            )
    }
}

impl OpenMusic {
    pub fn show_page(
        &self,
        _uri: &str,
        _card_side: Pixels,
        palette: Palette,
        _cx: &mut Context<Self>,
    ) -> AnyElement {
        self.page_status("Coming soon", palette).into_any_element()
    }

    pub fn episode_page(
        &self,
        _uri: &str,
        _card_side: Pixels,
        palette: Palette,
        _cx: &mut Context<Self>,
    ) -> AnyElement {
        self.page_status("Coming soon", palette).into_any_element()
    }

    pub fn your_episodes_page(
        &self,
        _card_side: Pixels,
        palette: Palette,
        _cx: &mut Context<Self>,
    ) -> AnyElement {
        self.page_status("Coming soon", palette).into_any_element()
    }

    pub fn now_playing_page(
        &self,
        _card_side: Pixels,
        palette: Palette,
        _cx: &mut Context<Self>,
    ) -> AnyElement {
        self.page_status("Coming soon", palette).into_any_element()
    }
}

pub fn menu_items(_target: &Target, _app: &OpenMusic) -> Vec<(Group, MenuItem)> {
    Vec::new()
}

pub fn nav_entries(app: &OpenMusic) -> Vec<NavEntry> {
    vec![NavEntry::new(
        "nav-lyrics",
        300,
        icons::LYRICS,
        "Lyrics",
        app.page == Page::Lyrics,
        |app, _, _| app.show_lyrics(true),
    )]
}

pub fn dock_items(
    app: &OpenMusic,
    _roomy: bool,
    palette: Palette,
    cx: &mut Context<OpenMusic>,
) -> Vec<DockItem> {
    let symbol = if app.page == Page::Lyrics {
        "quote.bubble.fill"
    } else {
        "quote.bubble"
    };
    vec![DockItem::new(
        "lyrics",
        700,
        48.,
        app.dock_control("lyrics", Label::Symbol(symbol), false, palette, cx)
            .size(px(48.)),
    )]
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
