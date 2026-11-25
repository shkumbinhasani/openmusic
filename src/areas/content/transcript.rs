use std::{cell::Cell, rc::Rc};

use gpui::{
    AnyElement, Context, FontWeight, ListAlignment, ListOffset, ListState, MouseButton, div, list,
    prelude::*, px, rgb, rgba,
};

use crate::{
    app::OpenMusic,
    ui::{
        Palette,
        components::{bone, timestamp},
        menu::Surface,
    },
};

const SIDE: f32 = 48.;
const OVERDRAW: f32 = 800.;
pub const SYNC_KEY: &str = "transcript-sync";

pub struct TranscriptView {
    pub uri: String,
    pub list: ListState,
    pub detached: Rc<Cell<bool>>,
}

impl TranscriptView {
    pub fn new(uri: String, lines: usize) -> Self {
        let list = ListState::new(lines + 2, ListAlignment::Top, px(OVERDRAW));
        let detached = Rc::new(Cell::new(false));
        let flag = detached.clone();
        list.set_scroll_handler(move |_, _, _| flag.set(true));
        Self {
            uri,
            list,
            detached,
        }
    }

    pub fn follow(&self, line: usize) {
        if self.detached.get() {
            return;
        }
        self.list.scroll_to(ListOffset {
            item_ix: (line + 1).saturating_sub(3),
            offset_in_item: px(0.),
        });
    }

    pub fn sync(&self, line: Option<usize>) {
        self.detached.set(false);
        match line {
            Some(line) => self.follow(line),
            None => self.list.scroll_to(ListOffset {
                item_ix: 0,
                offset_in_item: px(0.),
            }),
        }
    }
}

fn title_block(heading: String, palette: Palette) -> gpui::Div {
    div()
        .px(px(SIDE))
        .pt(px(40.))
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .text_size(px(30.))
                .font_weight(FontWeight::BOLD)
                .child("Transcript"),
        )
        .child(
            div()
                .text_color(rgb(palette.muted))
                .truncate()
                .child(heading),
        )
}

impl OpenMusic {
    pub fn sync_transcript(&mut self) {
        let line = self.content.lyrics_line;
        if let Some(view) = &self.content.transcript {
            view.sync(line);
        }
    }

    pub(super) fn transcript_page(&self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let Some(track) = self.track() else {
            return div().into_any_element();
        };
        let heading = if track.artist.is_empty() {
            track.title.clone()
        } else {
            format!("{} · {}", track.title, track.artist)
        };
        let menu = Rc::new(self.on_menu(None, |app| app.now_playing_target(Surface::Header), cx));
        let shown = self
            .content
            .lyrics
            .as_ref()
            .filter(|(uri, _)| *uri == track.uri)
            .and_then(|(_, lyrics)| lyrics.clone())
            .zip(
                self.content
                    .transcript
                    .as_ref()
                    .filter(|view| view.uri == track.uri),
            );
        let Some((lyrics, view)) = shown else {
            let header_menu = menu.clone();
            return div()
                .id("transcript")
                .size_full()
                .overflow_y_scroll()
                .pb(px(320.))
                .child(
                    title_block(heading, palette)
                        .on_mouse_down(MouseButton::Right, move |event, window, cx| {
                            header_menu(event, window, cx)
                        }),
                )
                .map(|page| {
                    if self.content.lyrics_loading {
                        page.child(
                            div()
                                .px(px(SIDE))
                                .pt_8()
                                .flex()
                                .flex_col()
                                .gap(px(18.))
                                .children(
                                    [0.7, 0.56, 0.64, 0.4, 0.6, 0.5]
                                        .into_iter()
                                        .enumerate()
                                        .map(|(index, width)| {
                                            bone("transcript-line", index, palette)
                                                .w(gpui::relative(width * 0.85))
                                                .max_w(px(640.))
                                                .h(px(22.))
                                                .rounded(px(6.))
                                        }),
                                ),
                        )
                    } else {
                        page.child(
                            div()
                                .px(px(SIDE))
                                .pt_6()
                                .text_size(px(17.))
                                .text_color(rgb(palette.muted))
                                .child("A transcript isn’t available for this episode."),
                        )
                    }
                })
                .into_any_element();
        };
        let synced = lyrics.synced;
        let active = self.content.lyrics_line;
        let count = lyrics.lines.len();
        let entity = cx.weak_entity();
        let rows = list(view.list.clone(), move |index, _, _| {
            if index == 0 {
                let header_menu = menu.clone();
                return title_block(heading.clone(), palette)
                    .pb_4()
                    .on_mouse_down(MouseButton::Right, move |event, window, cx| {
                        header_menu(event, window, cx)
                    })
                    .child(
                        div()
                            .pt_2()
                            .text_size(px(12.))
                            .text_color(rgb(palette.muted))
                            .child(lyrics.provider.clone()),
                    )
                    .into_any_element();
            }
            let Some(line) = lyrics.lines.get(index - 1) else {
                return div().h(px(320.)).into_any_element();
            };
            if line.heading {
                return div()
                    .px(px(SIDE + 12.))
                    .pt(px(if index > 1 { 20. } else { 4. }))
                    .pb_1()
                    .flex()
                    .gap_2()
                    .text_size(px(13.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(palette.muted))
                    .child(timestamp(line.start_ms))
                    .child(line.words.clone())
                    .into_any_element();
            }
            let current = active == Some(index - 1);
            let start = line.start_ms;
            let entity = entity.clone();
            div()
                .px(px(SIDE - 12.))
                .child(
                    div()
                        .id(("transcript-line", index))
                        .max_w(px(784.))
                        .px_3()
                        .py(px(6.))
                        .rounded(px(12.))
                        .text_size(px(22.))
                        .line_height(px(30.))
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
                                .on_click(move |_, _, cx| {
                                    let _ = entity.update(cx, |this, cx| this.seek_to(start, cx));
                                })
                        })
                        .child(line.words.clone()),
                )
                .into_any_element()
        })
        .size_full();
        let detached = synced && view.detached.get() && count > 0;
        div()
            .relative()
            .size_full()
            .child(rows)
            .when(detached, |page| {
                page.child(
                    div()
                        .absolute()
                        .top(px(16.))
                        .right(px(24.))
                        .child(self.text_control(SYNC_KEY, "Sync", true, palette, cx)),
                )
            })
            .into_any_element()
    }
}
