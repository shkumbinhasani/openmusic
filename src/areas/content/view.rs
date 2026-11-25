use std::time::Duration;

use gpui::{
    AnyElement, Context, Div, FontWeight, MouseButton, Pixels, SharedString, Stateful, div,
    prelude::*, px, relative, rgb, rgba,
};
use gpui_component::switch::Switch;
use spotify_client::{
    Track, entity,
    podcast::{
        self, Availability, Episode, Played, Show, ShowKind, similar, your_episodes::Filter,
    },
};

use super::state::{open_in_spotify, show_kind_label};
use crate::{
    app::{OpenMusic, Page, page_for},
    areas::account::state::EXPLICIT_EPISODE_NOTICE,
    icons,
    overlay::Label,
    ui::{
        Palette,
        components::{
            ROW_RADIUS, artwork, bone, explicit_badge, icon, link, scroll_sentinel,
            skeleton_header, skeleton_rows,
        },
        dock::DockItem,
        menu::{Group, MenuItem, Row, Surface, Target},
        sidebar::NavEntry,
        tracks::{RowOptions, now_ms},
    },
};

const ABOUT_CHARS: usize = 600;
const ABOUT_LINES: usize = 8;
const SETTING_INSET: f32 = 14.;
const CANVAS_TEXT: &str = "Short, looping visuals when a song is playing.";
const CANVAS_STILL: &str =
    "Reduce Motion is on in System Settings, so the cover shows instead of Canvas.";
const MUSIC_VIDEOS_TEXT: &str = "When off, music videos and live performances play as audio-only.";
const VIDEO_PODCASTS_TEXT: &str = "When off, video podcasts play as audio-only.";

fn settings_section(title: &'static str, rows: Vec<AnyElement>, palette: Palette) -> AnyElement {
    let count = rows.len();
    div()
        .flex()
        .flex_col()
        .gap_2p5()
        .child(
            div()
                .px_1()
                .text_size(px(16.))
                .font_weight(FontWeight::BOLD)
                .child(title),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .rounded(px(ROW_RADIUS + 4.))
                .bg(rgba(palette.press))
                .children(rows.into_iter().enumerate().flat_map(|(index, row)| {
                    let line = (index + 1 < count).then(|| {
                        div()
                            .h(px(1.))
                            .mx(px(SETTING_INSET))
                            .bg(rgba(palette.line))
                            .into_any_element()
                    });
                    std::iter::once(row).chain(line)
                })),
        )
        .into_any_element()
}

fn setting_row(
    title: &'static str,
    description: Vec<&'static str>,
    control: impl IntoElement,
    palette: Palette,
) -> AnyElement {
    div()
        .min_h(px(52.))
        .flex()
        .items_center()
        .gap_4()
        .px(px(SETTING_INSET))
        .py(px(10.))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap_0p5()
                .child(
                    div()
                        .text_size(px(13.5))
                        .font_weight(FontWeight::MEDIUM)
                        .child(title),
                )
                .children(description.into_iter().map(|line| {
                    div()
                        .text_size(px(12.))
                        .text_color(rgb(palette.muted))
                        .child(line)
                })),
        )
        .child(div().flex_shrink_0().flex().items_center().child(control))
        .into_any_element()
}

#[derive(Clone, Copy)]
pub(super) struct RowIds {
    pub row: &'static str,
    pub play: &'static str,
    pub save: &'static str,
    pub show: &'static str,
}

const SHOW_ROWS: RowIds = RowIds {
    row: "show-episode",
    play: "show-episode-play",
    save: "show-episode-save",
    show: "show-episode-show",
};

pub(super) fn hairline(palette: Palette) -> Div {
    div().h(px(1.)).mx_3().bg(rgba(palette.line))
}

pub(super) fn heading(text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(19.))
        .font_weight(FontWeight::BOLD)
        .child(text.into())
}

pub(super) fn kind_label(text: &'static str, palette: Palette) -> Div {
    div()
        .text_size(px(11.5))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(palette.accent))
        .child(text)
}

fn video_badge(palette: Palette) -> Div {
    div()
        .flex_shrink_0()
        .h(px(16.))
        .px(px(5.))
        .flex()
        .items_center()
        .rounded(px(4.))
        .bg(rgba(palette.press))
        .text_size(px(9.5))
        .font_weight(FontWeight::BOLD)
        .text_color(rgb(palette.muted))
        .child("VIDEO")
}

fn progress_bar(played: Played, duration_ms: u32, width: f32, palette: Palette) -> Div {
    let fraction = if duration_ms == 0 {
        0.
    } else {
        (played.position() as f32 / duration_ms as f32).clamp(0.02, 1.)
    };
    div()
        .flex_shrink_0()
        .w(px(width))
        .h(px(4.))
        .rounded_full()
        .bg(rgba(palette.line))
        .child(
            div()
                .h_full()
                .w(relative(fraction))
                .rounded_full()
                .bg(rgb(palette.accent)),
        )
}

fn finished_mark(text: &'static str, palette: Palette) -> Div {
    div()
        .flex_shrink_0()
        .flex()
        .items_center()
        .gap_1()
        .text_size(px(12.5))
        .text_color(rgb(palette.accent))
        .child(icon(icons::CHECK, 14., palette.accent))
        .child(text)
}

fn paragraphs(text: &str, size: f32, palette: Palette) -> Div {
    div()
        .max_w(px(760.))
        .flex()
        .flex_col()
        .text_size(px(size))
        .line_height(px(size * 1.57))
        .text_color(rgb(palette.muted))
        .children(text.lines().map(|line| {
            if line.is_empty() {
                div().h(px(size * 0.8))
            } else {
                div().child(line.to_owned())
            }
        }))
}

fn shortened(text: &str) -> Option<String> {
    let lines: Vec<&str> = text.lines().collect();
    if text.chars().count() <= ABOUT_CHARS && lines.len() <= ABOUT_LINES {
        return None;
    }
    let mut short = String::new();
    for line in lines.into_iter().take(ABOUT_LINES) {
        if !short.is_empty() {
            short.push('\n');
        }
        let room = ABOUT_CHARS.saturating_sub(short.chars().count());
        if line.chars().count() > room {
            short.extend(line.chars().take(room));
            break;
        }
        short.push_str(line);
    }
    Some(format!("{}…", short.trim_end()))
}

fn meta_line(parts: impl IntoIterator<Item = Option<String>>) -> String {
    parts
        .into_iter()
        .flatten()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" · ")
}

pub(super) fn this_year() -> i32 {
    podcast::year_of(now_ms())
}

fn episode_kind_label(episode: &Episode) -> &'static str {
    match (episode.chapter, episode.video) {
        (true, _) => "AUDIOBOOK CHAPTER",
        (false, true) => "VIDEO EPISODE",
        (false, false) => "PODCAST EPISODE",
    }
}

fn unplayable_note(episode: &Episode) -> Option<&'static str> {
    match episode.availability {
        Availability::Playable => None,
        Availability::Restricted => Some("This episode isn’t available in your country."),
        Availability::PaymentRequired if episode.chapter => {
            Some("Unlock this chapter in the Spotify app to listen.")
        }
        Availability::PaymentRequired => Some("This episode is for subscribers only."),
        Availability::Other => Some("This episode can’t play right now."),
        Availability::Missing => Some("Spotify couldn’t find this episode."),
    }
}

impl OpenMusic {
    pub fn display_section(&self, palette: Palette, cx: &mut Context<Self>) -> Option<AnyElement> {
        let switch = Switch::new("settings-canvas")
            .checked(self.settings.content.canvas)
            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                this.set_canvas(*checked);
                cx.notify();
            }));
        let mut description = vec![CANVAS_TEXT];
        if self.settings.content.canvas && self.content.canvas.reduce_motion {
            description.push(CANVAS_STILL);
        }
        let music = Switch::new("settings-music-videos")
            .checked(self.settings.content.music_videos)
            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                let on = *checked;
                this.update_settings(|settings| settings.content.music_videos = on);
                cx.notify();
            }));
        let podcasts = Switch::new("settings-video-podcasts")
            .checked(self.settings.content.video_podcasts)
            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                let on = *checked;
                this.update_settings(|settings| settings.content.video_podcasts = on);
                cx.notify();
            }));
        Some(settings_section(
            "Videos and Canvas",
            vec![
                setting_row("Canvas", description, switch, palette),
                setting_row("Music videos", vec![MUSIC_VIDEOS_TEXT], music, palette),
                setting_row(
                    "Video podcasts",
                    vec![VIDEO_PODCASTS_TEXT],
                    podcasts,
                    palette,
                ),
            ],
            palette,
        ))
    }

    pub(super) fn lyrics_retry(
        &self,
        id: &'static str,
        message: &'static str,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Div {
        div().flex().child(
            div()
                .id(id)
                .cursor_pointer()
                .text_size(px(17.))
                .text_color(rgb(palette.muted))
                .hover(move |retry| retry.text_color(rgb(palette.text)).underline())
                .child(format!("{message} Try again."))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.retry_lyrics(Duration::ZERO);
                    cx.notify();
                })),
        )
    }

    pub fn lyrics_page(&self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        if self
            .track()
            .is_some_and(|track| track.uri.starts_with("spotify:episode:"))
        {
            return self.transcript_page(palette, cx);
        }
        let track = self.track();
        let lyrics = self.current_lyrics();
        let synced = lyrics.is_some_and(|lyrics| lyrics.synced);
        let lines = lyrics
            .map(|lyrics| lyrics.lines.as_slice())
            .unwrap_or_default();
        let active = self.content.lyrics_line;
        let failed = self.lyrics_failed();
        let status = match (track, lyrics) {
            (None, _) => Some("Play a song to see its lyrics."),
            (Some(_), None) if self.content.lyrics_loading || failed => None,
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
                    })
                    .when(failed, |header| {
                        header.child(div().pt_6().child(self.lyrics_retry(
                            "lyrics-retry",
                            "Couldn’t load the lyrics.",
                            palette,
                            cx,
                        )))
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
            .into_any_element()
    }
}

impl OpenMusic {
    pub fn show_page(
        &self,
        uri: &str,
        card_side: Pixels,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let page = div()
            .flex()
            .flex_col()
            .gap_7()
            .child(div().child(self.back_button(palette, cx)));
        let Some((_, result)) = self
            .content
            .show
            .as_ref()
            .filter(|(loaded, _)| loaded == uri)
        else {
            return page
                .child(skeleton_header("show-header", false, palette))
                .child(skeleton_rows("show-rows", 6, false, palette))
                .into_any_element();
        };
        let show = match result {
            Ok(show) => show,
            Err(error) => {
                return page
                    .child(self.page_status(error.clone(), palette))
                    .into_any_element();
            }
        };
        let book = show.book.as_ref();
        let copyrights = book.map(|book| book.copyrights.clone()).unwrap_or_default();
        page.child(self.show_header(show, palette, cx))
            .when_some(self.access_note(show, palette), |page, note| {
                page.child(note)
            })
            .when_some(self.show_tags(show, palette, cx), |page, tags| {
                page.child(tags)
            })
            .when(!show.description.is_empty(), |page| {
                page.child(self.about(&show.description, palette, cx))
            })
            .child(self.episode_section(show, palette, cx))
            .when(!copyrights.is_empty(), |page| {
                page.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .text_size(px(11.5))
                        .text_color(rgb(palette.muted))
                        .children(copyrights.into_iter().map(|line| div().child(line))),
                )
            })
            .when_some(
                self.content
                    .similar
                    .as_ref()
                    .filter(|(loaded, _)| loaded == uri)
                    .and_then(|(_, cards)| cards.as_ref().ok())
                    .filter(|cards| !cards.is_empty()),
                |page, cards| {
                    page.child(self.shelf(
                        similar::title(show.kind),
                        None,
                        cards,
                        "show-similar",
                        card_side,
                        palette,
                        cx,
                    ))
                },
            )
            .into_any_element()
    }

    fn show_header(&self, show: &Show, palette: Palette, cx: &mut Context<Self>) -> Div {
        let image = show.cover.as_ref().and_then(|url| self.covers.get(url));
        let target = Target::Show {
            uri: show.uri.clone(),
        };
        let has_menu = self.has_menu(&target);
        let saved = self.saved_or_known(&show.uri).unwrap_or(false);
        let book = show.book.as_ref();
        let byline = self.byline(show, palette, cx);
        let narrators = book
            .map(|book| book.narrators.join(", "))
            .filter(|narrators| !narrators.is_empty());
        let rating = show.rating.map(podcast::rating_label);
        let facts = match book {
            Some(book) => meta_line([
                rating,
                entity::long_duration(book.duration_ms),
                book.published.as_deref().and_then(entity::short_date),
            ]),
            None => meta_line([
                rating,
                (show.total > 0).then(|| match show.total {
                    1 => "1 episode".to_owned(),
                    total => format!("{total} episodes"),
                }),
            ]),
        };
        let signifier = book
            .and_then(|book| book.access.as_ref())
            .and_then(|access| access.signifier.clone());
        let price = book.and_then(|book| book.price.clone());
        let actions = div().mt_3().flex().items_center().gap_2().map(|actions| {
            if show.kind == ShowKind::Audiobook {
                actions
                    .child(self.text_control("show-open", "Open in Spotify", true, palette, cx))
                    .child(
                        self.text_control(
                            "show-follow",
                            if saved { "Saved" } else { "Save" },
                            false,
                            palette,
                            cx,
                        )
                        .w(px(80.)),
                    )
            } else {
                let playing = self.show_playing(show);
                let paused_here = !self.playback.now.playing
                    && self.track().is_some_and(|track| {
                        show.episodes.iter().any(|episode| episode.uri == track.uri)
                    });
                let resume = self.show_resume(show);
                let label = if playing {
                    "Pause"
                } else if paused_here || resume.is_some_and(|(_, started)| started) {
                    "Resume"
                } else {
                    "Play"
                };
                actions
                    .when(playing || paused_here || resume.is_some(), |actions| {
                        actions.child(
                            self.text_control("show-play", label, true, palette, cx)
                                .w(px(96.)),
                        )
                    })
                    .child(
                        self.text_control(
                            "show-follow",
                            if saved { "Following" } else { "Follow" },
                            false,
                            palette,
                            cx,
                        )
                        .w(px(104.)),
                    )
            }
        });
        let menu_target = target.clone();
        div()
            .flex()
            .items_center()
            .gap_6()
            .on_mouse_down(
                MouseButton::Right,
                self.on_menu(None, move |_| Some(menu_target.clone()), cx),
            )
            .child(artwork(image, px(184.), px(16.), palette).shadow_lg())
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .items_start()
                    .gap_1p5()
                    .child(kind_label(show_kind_label(show), palette))
                    .child(
                        div()
                            .w_full()
                            .text_size(px(32.))
                            .line_height(px(38.))
                            .font_weight(FontWeight::BOLD)
                            .line_clamp(2)
                            .text_ellipsis()
                            .child(show.name.clone()),
                    )
                    .when_some(byline, |column, byline| column.child(byline))
                    .when_some(narrators, |column, narrators| {
                        column.child(
                            div()
                                .w_full()
                                .truncate()
                                .text_size(px(13.))
                                .text_color(rgb(palette.muted))
                                .child(format!("Narrated by {narrators}")),
                        )
                    })
                    .when(!facts.is_empty(), |column| {
                        column.child(
                            div()
                                .text_size(px(13.))
                                .text_color(rgb(palette.muted))
                                .child(facts),
                        )
                    })
                    .when(signifier.is_some() || price.is_some(), |column| {
                        column.child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .text_size(px(12.5))
                                .when_some(signifier, |line, signifier| {
                                    line.child(
                                        div()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(rgb(palette.accent))
                                            .child(signifier),
                                    )
                                })
                                .when_some(price, |line, price| {
                                    line.child(
                                        div().font_weight(FontWeight::SEMIBOLD).child(price.amount),
                                    )
                                    .when_some(
                                        price.list,
                                        |line, list| {
                                            line.child(
                                                div()
                                                    .line_through()
                                                    .text_color(rgb(palette.muted))
                                                    .child(list),
                                            )
                                        },
                                    )
                                }),
                        )
                    })
                    .child(actions.when(has_menu, |actions| {
                        actions.child(self.menu_button("more-show", &show.name, palette, cx))
                    })),
            )
    }

    fn byline(&self, show: &Show, palette: Palette, cx: &mut Context<Self>) -> Option<Div> {
        let line = div()
            .w_full()
            .min_w_0()
            .flex()
            .flex_wrap()
            .text_size(px(14.))
            .font_weight(FontWeight::SEMIBOLD);
        let authors = show
            .book
            .as_ref()
            .map(|book| book.authors.as_slice())
            .unwrap_or_default();
        if authors.is_empty() {
            return (!show.publisher.is_empty())
                .then(|| line.child(div().truncate().child(show.publisher.clone())));
        }
        Some(
            line.children(authors.iter().enumerate().map(|(index, author)| {
                let name = if index + 1 < authors.len() {
                    format!("{}, ", author.name)
                } else {
                    author.name.clone()
                };
                let uri = author.uri.clone();
                div()
                    .id(("book-author", index))
                    .flex_shrink_0()
                    .when(!uri.is_empty(), |name| {
                        name.cursor_pointer()
                            .hover(move |name| name.text_color(rgb(palette.text)).underline())
                            .on_click(cx.listener(move |_, _, _, cx| open_in_spotify(&uri, cx)))
                    })
                    .child(name)
            })),
        )
    }

    fn access_note(&self, show: &Show, palette: Palette) -> Option<Div> {
        let access = show.book.as_ref()?.access.as_ref()?;
        if access.title.is_none() && access.body.is_none() {
            return None;
        }
        Some(
            div()
                .max_w(px(760.))
                .flex()
                .flex_col()
                .gap_1()
                .p_4()
                .rounded(px(12.))
                .bg(rgba(palette.press))
                .when_some(access.title.clone(), |note, title| {
                    note.child(
                        div()
                            .text_size(px(14.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(title),
                    )
                })
                .when_some(access.body.clone(), |note, body| {
                    note.child(
                        div()
                            .text_size(px(13.))
                            .text_color(rgb(palette.muted))
                            .child(body),
                    )
                }),
        )
    }

    fn show_tags(&self, show: &Show, palette: Palette, cx: &mut Context<Self>) -> Option<Div> {
        if !show.topics.is_empty() {
            let chips: Vec<SharedString> = show
                .topics
                .iter()
                .map(|topic| topic.title.clone().into())
                .collect();
            let pages: Vec<Option<Page>> = show
                .topics
                .iter()
                .map(|topic| page_for(&topic.uri))
                .collect();
            return Some(self.chip_row(
                "show-topic",
                &chips,
                None,
                move |app, index, _, _| {
                    if let Some(Some(page)) = pages.get(index) {
                        app.navigate(page.clone());
                    }
                },
                palette,
                cx,
            ));
        }
        let genres = &show.book.as_ref()?.genres;
        if genres.is_empty() {
            return None;
        }
        Some(
            div()
                .flex()
                .flex_wrap()
                .gap_2()
                .children(genres.iter().map(|genre| {
                    div()
                        .h(px(28.))
                        .px_3()
                        .flex()
                        .items_center()
                        .rounded_full()
                        .border_1()
                        .border_color(rgba(palette.line))
                        .text_size(px(12.5))
                        .font_weight(FontWeight::MEDIUM)
                        .child(genre.clone())
                })),
        )
    }

    fn about(&self, text: &str, palette: Palette, cx: &mut Context<Self>) -> Div {
        let short = shortened(text);
        let open = self.content.about_open;
        let shown = match (&short, open) {
            (Some(short), false) => short.as_str(),
            _ => text,
        };
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(heading("About"))
            .child(paragraphs(shown, 14., palette))
            .when(short.is_some(), |about| {
                about.child(
                    div().flex().child(
                        div()
                            .id("show-about-toggle")
                            .cursor_pointer()
                            .text_size(px(13.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .hover(|toggle| toggle.underline())
                            .child(if open { "Show less" } else { "Show more" })
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.content.about_open = !this.content.about_open;
                                cx.notify();
                            })),
                    ),
                )
            })
    }

    fn episode_section(&self, show: &Show, palette: Palette, cx: &mut Context<Self>) -> Div {
        let book = show.kind == ShowKind::Audiobook;
        let year = this_year();
        let filter = self.content.show_filter.filter(|_| !book);
        let visible: Vec<(usize, &Episode)> = show
            .episodes
            .iter()
            .enumerate()
            .filter(|(_, episode)| {
                episode.listed()
                    && filter.is_none_or(|filter| filter.keeps(episode, self.played(episode)))
            })
            .collect();
        let restricted = show.restricted();
        let loading = self.content.show_loading;
        let error = self.content.show_error.clone();
        let count = match (book, show.total) {
            (_, 0) => String::new(),
            (true, 1) => "1 chapter".into(),
            (true, total) => format!("{total} chapters"),
            (false, 1) => "1 episode".into(),
            (false, total) => format!("{total} episodes"),
        };
        let mut list = div().flex().flex_col();
        for (position, (index, episode)) in visible.iter().enumerate() {
            if book {
                list = list.child(self.chapter_row(show, *index, episode, palette, cx));
            } else {
                if position > 0 {
                    list = list.child(hairline(palette));
                }
                let index = *index;
                list = list.child(self.episode_row(
                    SHOW_ROWS,
                    index,
                    episode,
                    false,
                    move |app, cx| app.play_show_episode(index, cx),
                    year,
                    palette,
                    cx,
                ));
            }
        }
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .flex()
                    .items_end()
                    .justify_between()
                    .gap_4()
                    .child(heading(if book { "Chapters" } else { "All Episodes" }))
                    .child(
                        div()
                            .text_size(px(13.))
                            .text_color(rgb(palette.muted))
                            .child(count),
                    ),
            )
            .when(!book && !show.episodes.is_empty(), |section| {
                section.child(self.show_filters(show, filter, palette, cx))
            })
            .when(
                visible.is_empty() && !loading && show.next_offset.is_none(),
                |section| {
                    section.child(self.page_status(
                        match filter {
                            Some(filter) => {
                                format!("No {} episodes.", filter.label().to_lowercase())
                            }
                            None if restricted > 0 => {
                                "These episodes aren’t available in your country.".to_owned()
                            }
                            None => "No episodes yet.".to_owned(),
                        },
                        palette,
                    ))
                },
            )
            .child(list)
            .when(loading, |section| {
                section.child(skeleton_rows("show-more-rows", 3, book, palette))
            })
            .when_some(error, |section, error| {
                section.child(
                    div().flex().child(
                        div()
                            .id("show-retry")
                            .py_2()
                            .cursor_pointer()
                            .text_size(px(13.))
                            .text_color(rgb(palette.muted))
                            .hover(move |retry| retry.text_color(rgb(palette.text)).underline())
                            .child(format!("{error} Try again."))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.retry_episodes();
                                cx.notify();
                            })),
                    ),
                )
            })
            .when(
                show.next_offset.is_some() && !loading && self.content.show_error.is_none(),
                |section| section.child(scroll_sentinel(&self.content.show_end)),
            )
            .when(restricted > 0 && !visible.is_empty(), |section| {
                section.child(
                    div()
                        .text_size(px(12.5))
                        .text_color(rgb(palette.muted))
                        .child(match restricted {
                            1 => "1 episode isn’t available in your country.".to_owned(),
                            count => format!("{count} episodes aren’t available in your country."),
                        }),
                )
            })
    }

    fn show_filters(
        &self,
        show: &Show,
        filter: Option<Filter>,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Div {
        let filters: Vec<Filter> = Filter::ALL
            .into_iter()
            .filter(|filter| *filter != Filter::Video || show.video())
            .collect();
        let chips: Vec<SharedString> = filters
            .iter()
            .map(|filter| SharedString::from(filter.label()))
            .collect();
        let selected = filter.and_then(|filter| filters.iter().position(|each| *each == filter));
        self.chip_row(
            "show-filter",
            &chips,
            selected,
            move |app, index, _, cx| {
                let picked = filters.get(index).copied();
                app.content.show_filter = if app.content.show_filter == picked {
                    None
                } else {
                    picked
                };
                cx.notify();
            },
            palette,
            cx,
        )
    }

    fn episode_play_button(
        &self,
        id: impl Into<gpui::ElementId>,
        episode: &Episode,
        on_play: impl Fn(&mut OpenMusic, &mut Context<OpenMusic>) + 'static,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let playing = self.is_current(&episode.uri) && self.playback.now.playing;
        let button = div()
            .id(id)
            .flex_shrink_0()
            .size(px(32.))
            .rounded_full()
            .flex()
            .items_center()
            .justify_center();
        let filtered = self.episode_filtered(episode) && !self.is_current(&episode.uri);
        if !episode.playable() || filtered {
            return button
                .bg(rgba(palette.press))
                .child(icon(icons::LOCK, 14., palette.muted));
        }
        button
            .bg(rgb(palette.accent))
            .cursor_pointer()
            .hover(|button| button.opacity(0.85))
            .child(icon(
                if playing { icons::PAUSE } else { icons::PLAY },
                14.,
                0xffffff,
            ))
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                on_play(this, cx);
                cx.notify();
            }))
    }

    fn episode_save_button(
        &self,
        id: impl Into<gpui::ElementId>,
        uri: &str,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let saved = self.is_saved(uri) == Some(true);
        let busy = self.saving(uri);
        let uri = uri.to_owned();
        div()
            .id(id)
            .flex_shrink_0()
            .size(px(32.))
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .map(|button| {
                if saved {
                    button
                        .bg(rgb(palette.accent))
                        .child(icon(icons::CHECK, 14., 0xffffff))
                } else {
                    button
                        .border_1()
                        .border_color(rgba(palette.line))
                        .child(icon(icons::PLUS, 14., palette.muted))
                }
            })
            .when(busy, |button| button.opacity(0.6))
            .hover(move |button| {
                if saved {
                    button.opacity(0.85)
                } else {
                    button.bg(rgba(palette.press))
                }
            })
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                this.toggle_saved_uri(&uri);
                cx.notify();
            }))
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn episode_row(
        &self,
        ids: RowIds,
        index: usize,
        episode: &Episode,
        with_show: bool,
        on_play: impl Fn(&mut OpenMusic, &mut Context<OpenMusic>) + 'static,
        year: i32,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let id = (ids.row, index);
        let played = self.played(episode);
        let current = self.is_current(&episode.uri);
        let image = episode.cover.as_ref().and_then(|url| self.covers.get(url));
        let summary = episode.summary();
        let meta = meta_line([
            episode
                .released
                .as_deref()
                .and_then(|released| podcast::release_label(released, year)),
            podcast::length_label(episode.duration_ms, played),
        ]);
        let open_uri = episode.uri.clone();
        let menu_uri = episode.uri.clone();
        div()
            .id(id)
            .flex()
            .gap_4()
            .p_3()
            .rounded(px(12.))
            .cursor_pointer()
            .when(!self.episode_startable(episode), |row| row.opacity(0.6))
            .when(self.menu_row(id), |row| row.bg(rgba(palette.press)))
            .hover(move |row| row.bg(rgba(palette.press)))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.navigate(Page::Episode(open_uri.clone()));
                cx.notify();
            }))
            .on_mouse_down(
                MouseButton::Right,
                self.on_menu(
                    Some(id),
                    move |_| {
                        Some(Target::Episode {
                            uri: menu_uri.clone(),
                        })
                    },
                    cx,
                ),
            )
            .child(artwork(image, px(88.), px(10.), palette))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_size(px(15.))
                            .line_height(px(20.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .line_clamp(2)
                            .text_ellipsis()
                            .when(current, |title| title.text_color(rgb(palette.accent)))
                            .child(episode.name.clone()),
                    )
                    .when(with_show && !episode.show_name.is_empty(), |column| {
                        column.child(div().flex().min_w_0().child(link(
                            (ids.show, index),
                            episode.show_name.clone(),
                            episode.show_uri.clone().map(Page::Show),
                            13.,
                            palette,
                            cx,
                        )))
                    })
                    .child(
                        div()
                            .flex()
                            .items_start()
                            .gap_1p5()
                            .min_w_0()
                            .when(episode.explicit, |line| {
                                line.child(div().pt(px(2.)).child(explicit_badge(palette)))
                            })
                            .when(episode.video, |line| {
                                line.child(div().pt(px(2.)).child(video_badge(palette)))
                            })
                            .when(!summary.is_empty(), |line| {
                                line.child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .text_size(px(13.))
                                        .line_height(px(19.))
                                        .text_color(rgb(palette.muted))
                                        .line_clamp(2)
                                        .text_ellipsis()
                                        .child(summary),
                                )
                            }),
                    )
                    .child(
                        div()
                            .mt_1()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(self.episode_play_button(
                                (ids.play, index),
                                episode,
                                on_play,
                                palette,
                                cx,
                            ))
                            .when(episode.listed(), |line| {
                                line.child(self.episode_save_button(
                                    (ids.save, index),
                                    &episode.uri,
                                    palette,
                                    cx,
                                ))
                            })
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(12.5))
                                    .text_color(rgb(palette.muted))
                                    .child(meta),
                            )
                            .when(played.started(), |line| {
                                line.child(progress_bar(played, episode.duration_ms, 96., palette))
                            })
                            .when(played.finished(), |line| {
                                line.child(finished_mark("Played", palette))
                            }),
                    ),
            )
    }

    fn chapter_row(
        &self,
        show: &Show,
        index: usize,
        episode: &Episode,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let id = ("show-chapter", index);
        let played = self.played(episode);
        let locked = !episode.playable();
        let book_uri = show.uri.clone();
        let menu_uri = episode.uri.clone();
        div()
            .id(id)
            .h(px(56.))
            .flex()
            .items_center()
            .gap_3()
            .px_2()
            .rounded(px(10.))
            .cursor_pointer()
            .when(self.menu_row(id), |row| row.bg(rgba(palette.press)))
            .hover(move |row| row.bg(rgba(palette.press)))
            .on_click(cx.listener(move |_, _, _, cx| open_in_spotify(&book_uri, cx)))
            .on_mouse_down(
                MouseButton::Right,
                self.on_menu(
                    Some(id),
                    move |_| {
                        Some(Target::Episode {
                            uri: menu_uri.clone(),
                        })
                    },
                    cx,
                ),
            )
            .child(
                div()
                    .w(px(40.))
                    .flex_shrink_0()
                    .text_center()
                    .text_size(px(13.))
                    .text_color(rgb(palette.muted))
                    .child((index + 1).to_string()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap_0p5()
                    .child(
                        div()
                            .text_size(px(13.5))
                            .font_weight(FontWeight::MEDIUM)
                            .truncate()
                            .child(episode.name.clone()),
                    )
                    .when(played.finished(), |column| {
                        column.child(finished_mark("Finished", palette))
                    })
                    .when(played.started(), |column| {
                        column.child(div().pt_1().child(progress_bar(
                            played,
                            episode.duration_ms,
                            96.,
                            palette,
                        )))
                    }),
            )
            .when(locked, |row| {
                row.child(icon(icons::LOCK, 14., palette.muted))
            })
            .child(
                div()
                    .w(px(72.))
                    .flex_shrink_0()
                    .text_right()
                    .text_size(px(12.5))
                    .text_color(rgb(palette.muted))
                    .child(podcast::length_label(episode.duration_ms, played).unwrap_or_default()),
            )
    }

    pub fn episode_page(
        &self,
        uri: &str,
        _card_side: Pixels,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let page = div()
            .flex()
            .flex_col()
            .gap_6()
            .child(div().child(self.back_button(palette, cx)));
        let Some((_, result)) = self
            .content
            .episode
            .as_ref()
            .filter(|(loaded, _)| loaded == uri)
        else {
            return page
                .child(skeleton_header("episode-header", false, palette))
                .child(skeleton_rows("episode-rows", 4, false, palette))
                .into_any_element();
        };
        let episode = match result {
            Ok(episode) => episode,
            Err(error) => {
                return page
                    .child(self.page_status(error.clone(), palette))
                    .into_any_element();
            }
        };
        let image = episode
            .large_cover
            .as_ref()
            .or(episode.cover.as_ref())
            .and_then(|url| self.covers.get(url));
        let played = self.played(episode);
        let current = self.is_current(&episode.uri);
        let playing = current && self.playback.now.playing;
        let filtered = self.episode_filtered(episode);
        let target = Target::Episode {
            uri: episode.uri.clone(),
        };
        let has_menu = self.has_menu(&target);
        let saved = self.is_saved(&episode.uri).unwrap_or(false);
        let show_page = episode.show_uri.clone().map(Page::Show);
        let meta = meta_line([
            episode
                .released
                .as_deref()
                .and_then(|released| podcast::release_label(released, this_year())),
            podcast::length_label(episode.duration_ms, played),
        ]);
        let play_label = if playing {
            "Pause"
        } else if current || played.started() {
            "Resume"
        } else {
            "Play"
        };
        let menu_target = target.clone();
        page.child(
            div()
                .flex()
                .items_center()
                .gap_6()
                .on_mouse_down(
                    MouseButton::Right,
                    self.on_menu(None, move |_| Some(menu_target.clone()), cx),
                )
                .child(artwork(image, px(184.), px(16.), palette).shadow_lg())
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .items_start()
                        .gap_1p5()
                        .child(kind_label(episode_kind_label(episode), palette))
                        .child(
                            div()
                                .w_full()
                                .text_size(px(28.))
                                .line_height(px(34.))
                                .font_weight(FontWeight::BOLD)
                                .line_clamp(3)
                                .text_ellipsis()
                                .child(episode.name.clone()),
                        )
                        .when(!episode.show_name.is_empty(), |column| {
                            column.child(div().w_full().flex().min_w_0().child(link(
                                ("episode-show-link", 0usize),
                                episode.show_name.clone(),
                                show_page,
                                14.,
                                palette,
                                cx,
                            )))
                        }),
                ),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap_4()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_3()
                        .when(episode.explicit, |line| line.child(explicit_badge(palette)))
                        .when(episode.video, |line| line.child(video_badge(palette)))
                        .child(
                            div()
                                .text_size(px(13.))
                                .text_color(rgb(palette.muted))
                                .child(meta),
                        )
                        .when(played.started(), |line| {
                            line.child(progress_bar(played, episode.duration_ms, 160., palette))
                        })
                        .when(played.finished(), |line| {
                            line.child(finished_mark(
                                if episode.chapter {
                                    "Finished"
                                } else {
                                    "Played"
                                },
                                palette,
                            ))
                        }),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .map(|actions| {
                            if episode.chapter {
                                actions.child(self.text_control(
                                    "episode-open",
                                    "Open in Spotify",
                                    true,
                                    palette,
                                    cx,
                                ))
                            } else {
                                actions
                                    .when(episode.playable() && (current || !filtered), |actions| {
                                        actions.child(
                                            self.text_control(
                                                "episode-play",
                                                play_label,
                                                true,
                                                palette,
                                                cx,
                                            )
                                            .w(px(96.)),
                                        )
                                    })
                                    .child(
                                        self.text_control(
                                            "episode-save",
                                            if saved { "Saved" } else { "Save" },
                                            false,
                                            palette,
                                            cx,
                                        )
                                        .w(px(80.)),
                                    )
                                    .child(self.text_control(
                                        "episode-played",
                                        if played.finished() {
                                            "Mark as not started"
                                        } else {
                                            "Mark as finished"
                                        },
                                        false,
                                        palette,
                                        cx,
                                    ))
                            }
                        })
                        .when(has_menu, |actions| {
                            actions.child(self.menu_button(
                                "more-episode",
                                &episode.name,
                                palette,
                                cx,
                            ))
                        }),
                )
                .when_some(
                    unplayable_note(episode).or(filtered.then_some(EXPLICIT_EPISODE_NOTICE)),
                    |section, note| {
                        section.child(
                            div()
                                .text_size(px(13.))
                                .text_color(rgb(palette.muted))
                                .child(note),
                        )
                    },
                ),
        )
        .when(!episode.description.is_empty(), |page| {
            page.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(heading("Episode Description"))
                    .child(paragraphs(&episode.description, 14., palette)),
            )
        })
        .when(episode.show_uri.is_some(), |page| {
            page.child(div().flex().child(self.text_control(
                "episode-show",
                if episode.chapter {
                    "See all chapters"
                } else {
                    "See all episodes"
                },
                false,
                palette,
                cx,
            )))
        })
        .into_any_element()
    }

    pub fn now_playing_page(
        &self,
        _card_side: Pixels,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.screen_page(palette, cx)
    }
}

pub fn menu_items(target: &Target, app: &OpenMusic) -> Vec<(Group, MenuItem)> {
    let choices = match target {
        Target::Episode { uri } => episode_choices(&episode_facts(app, uri, None), &app.page),
        Target::Track(track) if track.track.uri.starts_with("spotify:episode:") => {
            let linked = linked_show(&track.track);
            episode_choices(&episode_facts(app, &track.track.uri, linked), &app.page)
        }
        Target::Show { uri } => show_choices(&show_facts(app, uri), &app.page),
        _ => Vec::new(),
    };
    choices
        .into_iter()
        .map(|(group, choice)| (group, menu_item(choice)))
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shelf {
    Episodes,
    Follow,
    Library,
}

#[derive(Clone, Debug, PartialEq)]
enum Choice {
    Save {
        uri: String,
        shelf: Shelf,
        saved: bool,
        busy: bool,
    },
    Played {
        uri: String,
        finished: bool,
        busy: bool,
    },
    GoToShow {
        episode: String,
        show: Option<String>,
    },
    GoToBook(String),
    Open(String),
}

#[derive(Debug, Default)]
struct EpisodeFacts {
    uri: String,
    show: Option<String>,
    chapter: bool,
    saved: bool,
    saving: bool,
    finished: bool,
    marking: bool,
}

#[derive(Debug, Default)]
struct ShowFacts {
    uri: String,
    audiobook: bool,
    saved: bool,
    saving: bool,
}

fn linked_show(track: &Track) -> Option<String> {
    track
        .artists
        .iter()
        .map(|link| link.uri.as_str())
        .chain(track.artist_uri.as_deref())
        .chain(track.album_uri.as_deref())
        .find(|uri| uri.starts_with("spotify:show:"))
        .map(str::to_owned)
}

fn episode_facts(app: &OpenMusic, uri: &str, linked: Option<String>) -> EpisodeFacts {
    let show = app.content.show_of(uri).or(linked);
    let chapter = app
        .content
        .loaded_episode(uri)
        .is_some_and(|episode| episode.chapter)
        || show
            .as_deref()
            .is_some_and(|show| app.show_kind(show) == Some(ShowKind::Audiobook));
    EpisodeFacts {
        uri: uri.to_owned(),
        show,
        chapter,
        saved: app.saved_or_known(uri) == Some(true),
        saving: app.saving(uri),
        finished: app.played_uri(uri).is_some_and(Played::finished),
        marking: app.content.marking.contains_key(uri),
    }
}

fn show_facts(app: &OpenMusic, uri: &str) -> ShowFacts {
    ShowFacts {
        uri: uri.to_owned(),
        audiobook: app.show_kind(uri) == Some(ShowKind::Audiobook),
        saved: app.saved_or_known(uri) == Some(true),
        saving: app.saving(uri),
    }
}

fn showing(page: &Page, uri: &str) -> bool {
    matches!(page, Page::Show(open) if open == uri)
}

fn episode_choices(facts: &EpisodeFacts, page: &Page) -> Vec<(Group, Choice)> {
    if facts.chapter {
        let book = facts.show.clone();
        let mut choices = Vec::new();
        if let Some(book) = book.as_ref().filter(|book| !showing(page, book)) {
            choices.push((Group::Navigate, Choice::GoToBook(book.clone())));
        }
        choices.push((
            Group::Navigate,
            Choice::Open(book.unwrap_or_else(|| facts.uri.clone())),
        ));
        return choices;
    }
    let mut choices = vec![
        (
            Group::Library,
            Choice::Save {
                uri: facts.uri.clone(),
                shelf: Shelf::Episodes,
                saved: facts.saved,
                busy: facts.saving,
            },
        ),
        (
            Group::Library,
            Choice::Played {
                uri: facts.uri.clone(),
                finished: facts.finished,
                busy: facts.marking,
            },
        ),
    ];
    if !facts
        .show
        .as_deref()
        .is_some_and(|show| showing(page, show))
    {
        choices.push((
            Group::Navigate,
            Choice::GoToShow {
                episode: facts.uri.clone(),
                show: facts.show.clone(),
            },
        ));
    }
    choices
}

fn show_choices(facts: &ShowFacts, page: &Page) -> Vec<(Group, Choice)> {
    let save = Choice::Save {
        uri: facts.uri.clone(),
        shelf: if facts.audiobook {
            Shelf::Library
        } else {
            Shelf::Follow
        },
        saved: facts.saved,
        busy: facts.saving,
    };
    let mut choices = vec![(Group::Library, save)];
    if facts.audiobook {
        if !showing(page, &facts.uri) {
            choices.push((Group::Navigate, Choice::GoToBook(facts.uri.clone())));
        }
        choices.push((Group::Navigate, Choice::Open(facts.uri.clone())));
    }
    choices
}

fn choice_label(choice: &Choice) -> (&'static str, &'static str, Option<&'static str>) {
    match choice {
        Choice::Save { shelf, saved, .. } => {
            let (title, key) = match (shelf, saved) {
                (Shelf::Episodes, false) => ("Save to Your Episodes", "s"),
                (Shelf::Episodes, true) => ("Remove from Your Episodes", "s"),
                (Shelf::Follow, false) => ("Follow", "f"),
                (Shelf::Follow, true) => ("Unfollow", "f"),
                (Shelf::Library, false) => ("Add to Your Library", "s"),
                (Shelf::Library, true) => ("Remove from Your Library", "s"),
            };
            let symbol = if *saved {
                "minus.circle"
            } else {
                "plus.circle"
            };
            (title, symbol, Some(key))
        }
        Choice::Played { finished: true, .. } => {
            ("Mark as not started", "arrow.counterclockwise.circle", None)
        }
        Choice::Played {
            finished: false, ..
        } => ("Mark as finished", "checkmark.circle", None),
        Choice::GoToShow { .. } => ("Go to podcast", "mic", None),
        Choice::GoToBook(_) => ("Go to audiobook", "book", None),
        Choice::Open(_) => ("Open in Spotify", "arrow.up.forward.app", None),
    }
}

fn menu_item(choice: Choice) -> MenuItem {
    let (title, symbol, key) = choice_label(&choice);
    let item = match choice {
        Choice::Save {
            uri, saved, busy, ..
        } => {
            MenuItem::action(title, move |app, _, _| app.change_saved(&uri, !saved)).enabled(!busy)
        }
        Choice::Played {
            uri,
            finished,
            busy,
        } => MenuItem::action(title, move |app, _, _| {
            app.mark_played(vec![uri.clone()], !finished)
        })
        .enabled(!busy),
        Choice::GoToShow { episode, show } => MenuItem::action(title, move |app, _, _| {
            app.go_to_show(&episode, show.clone())
        }),
        Choice::GoToBook(book) => MenuItem::action(title, move |app, _, _| {
            app.navigate(Page::Show(book.clone()))
        }),
        Choice::Open(uri) => MenuItem::action(title, move |_, _, cx| open_in_spotify(&uri, cx)),
    };
    let item = item.symbol(symbol);
    match key {
        Some(key) => item.key(key),
        None => item,
    }
}

pub fn nav_entries(app: &OpenMusic) -> Vec<NavEntry> {
    vec![
        NavEntry::new(
            "nav-lyrics",
            300,
            icons::LYRICS,
            "Lyrics",
            app.page == Page::Lyrics,
            |app, _, _| app.show_lyrics(true),
        ),
        NavEntry::new(
            "nav-now-playing",
            310,
            icons::NOTE,
            "Now Playing",
            app.page == Page::NowPlaying,
            |app, _, _| {
                if app.page != Page::NowPlaying {
                    app.navigate(Page::NowPlaying);
                }
            },
        ),
    ]
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

pub fn row_dimmed(app: &OpenMusic, track: &Track, _row: Row, _options: &RowOptions) -> bool {
    app.content.unavailable.contains(&track.uri)
}

#[cfg(test)]
mod tests {
    use spotify_client::catalog::Link;

    use super::*;

    const EPISODE: &str = "spotify:episode:00000000000000000000e1";
    const SHOW: &str = "spotify:show:00000000000000000000s1";
    const BOOK: &str = "spotify:show:00000000000000000000b1";

    fn titles(choices: &[(Group, Choice)]) -> Vec<(Group, &'static str)> {
        choices
            .iter()
            .map(|(group, choice)| (*group, choice_label(choice).0))
            .collect()
    }

    fn episode(show: Option<&str>) -> EpisodeFacts {
        EpisodeFacts {
            uri: EPISODE.into(),
            show: show.map(str::to_owned),
            ..EpisodeFacts::default()
        }
    }

    #[test]
    fn episodes_save_mark_and_lead_to_their_podcast() {
        let choices = episode_choices(&episode(Some(SHOW)), &Page::Home);
        assert_eq!(
            titles(&choices),
            vec![
                (Group::Library, "Save to Your Episodes"),
                (Group::Library, "Mark as finished"),
                (Group::Navigate, "Go to podcast"),
            ]
        );
        assert_eq!(
            choices[2].1,
            Choice::GoToShow {
                episode: EPISODE.into(),
                show: Some(SHOW.into()),
            }
        );

        let on_show = episode_choices(&episode(Some(SHOW)), &Page::Show(SHOW.into()));
        assert_eq!(on_show.len(), 2);

        let unknown = episode_choices(&episode(None), &Page::Show(SHOW.into()));
        assert_eq!(
            unknown[2].1,
            Choice::GoToShow {
                episode: EPISODE.into(),
                show: None,
            }
        );
    }

    #[test]
    fn episode_items_follow_saved_finished_and_busy_state() {
        let facts = EpisodeFacts {
            saved: true,
            finished: true,
            saving: true,
            ..episode(Some(SHOW))
        };
        let choices = episode_choices(&facts, &Page::Episode(EPISODE.into()));
        assert_eq!(
            titles(&choices)[..2],
            [
                (Group::Library, "Remove from Your Episodes"),
                (Group::Library, "Mark as not started"),
            ]
        );
        assert!(matches!(choices[0].1, Choice::Save { busy: true, .. }));
        assert!(matches!(choices[1].1, Choice::Played { busy: false, .. }));
        assert_eq!(choice_label(&choices[0].1).2, Some("s"));
    }

    #[test]
    fn chapters_lead_to_their_book() {
        let facts = EpisodeFacts {
            chapter: true,
            ..episode(Some(BOOK))
        };
        assert_eq!(
            episode_choices(&facts, &Page::Home),
            vec![
                (Group::Navigate, Choice::GoToBook(BOOK.into())),
                (Group::Navigate, Choice::Open(BOOK.into())),
            ]
        );
        assert_eq!(
            episode_choices(&facts, &Page::Show(BOOK.into())),
            vec![(Group::Navigate, Choice::Open(BOOK.into()))]
        );
        let loose = EpisodeFacts {
            chapter: true,
            ..episode(None)
        };
        assert_eq!(
            episode_choices(&loose, &Page::Home),
            vec![(Group::Navigate, Choice::Open(EPISODE.into()))]
        );
    }

    #[test]
    fn podcasts_follow_and_books_go_to_their_page() {
        let podcast = ShowFacts {
            uri: SHOW.into(),
            ..ShowFacts::default()
        };
        let choices = show_choices(&podcast, &Page::Home);
        assert_eq!(titles(&choices), vec![(Group::Library, "Follow")]);
        assert_eq!(choice_label(&choices[0].1).2, Some("f"));
        let followed = ShowFacts {
            saved: true,
            ..podcast
        };
        assert_eq!(
            titles(&show_choices(&followed, &Page::Home)),
            vec![(Group::Library, "Unfollow")]
        );

        let book = ShowFacts {
            uri: BOOK.into(),
            audiobook: true,
            ..ShowFacts::default()
        };
        assert_eq!(
            titles(&show_choices(&book, &Page::Home)),
            vec![
                (Group::Library, "Add to Your Library"),
                (Group::Navigate, "Go to audiobook"),
                (Group::Navigate, "Open in Spotify"),
            ]
        );
        assert_eq!(
            titles(&show_choices(&book, &Page::Show(BOOK.into()))),
            vec![
                (Group::Library, "Add to Your Library"),
                (Group::Navigate, "Open in Spotify"),
            ]
        );
    }

    #[test]
    fn finds_the_show_an_episode_row_links_to() {
        let mut track = Track {
            uri: EPISODE.into(),
            artists: vec![Link {
                uri: SHOW.into(),
                name: "A show".into(),
            }],
            ..Track::default()
        };
        assert_eq!(linked_show(&track).as_deref(), Some(SHOW));
        track.artists.clear();
        track.album_uri = Some(SHOW.into());
        assert_eq!(linked_show(&track).as_deref(), Some(SHOW));
        track.album_uri = Some("spotify:album:00000000000000000000a1".into());
        assert_eq!(linked_show(&track), None);
    }
}
