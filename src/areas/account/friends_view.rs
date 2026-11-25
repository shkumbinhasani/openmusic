use std::time::Instant;

use gpui::{
    AnyElement, Context, Div, FontWeight, MouseButton, Stateful, div, prelude::*, px, rgb, rgba,
};
use spotify_client::social::Friend;

use super::friends::{PANEL_WIDTH, ago};
use crate::{
    app::{OpenMusic, Page, page_for},
    icons,
    overlay::{self, Label},
    ui::{
        Palette, Panel,
        components::{ROW_RADIUS, bone, icon, link},
        menu::{Surface, Target},
        tracks::now_ms,
    },
};

pub const HIDE_KEY: &str = "friends-hide";
pub const RETRY_KEY: &str = "friends-retry";
pub const ROW: &str = "friend";
const EMPTY_TITLE: &str = "No activity yet";
const EMPTY_TEXT: &str =
    "When people you follow share what they’re listening to, it shows up here.";

impl OpenMusic {
    pub fn friends_panel(&self, palette: Palette, cx: &mut Context<Self>) -> Option<Panel> {
        if !self.friends_on() {
            return None;
        }
        self.account.friends.drawn.set(Some(Instant::now()));
        let element = palette
            .glass("friend-activity", 0.)
            .size_full()
            .flex()
            .flex_col()
            .gap_3()
            .pt(px(48.))
            .pb(px(10.))
            .px(px(10.))
            .child(self.friends_header(palette, cx))
            .child(self.friends_body(palette, cx));
        Some(Panel {
            width: PANEL_WIDTH,
            element: element.into_any_element(),
        })
    }

    fn friends_header(&self, palette: Palette, cx: &mut Context<Self>) -> Div {
        let hide = if overlay::available() {
            self.control(HIDE_KEY, Label::Symbol("xmark"), false, palette, cx)
                .size(px(28.))
                .into_any_element()
        } else {
            div()
                .id(HIDE_KEY)
                .text_size(px(12.5))
                .text_color(rgb(palette.muted))
                .cursor_pointer()
                .hover(|hide| hide.underline())
                .child("Hide")
                .on_click(cx.listener(|this, _, _, cx| {
                    this.set_friend_activity(false);
                    cx.notify();
                }))
                .into_any_element()
        };
        div()
            .flex_shrink_0()
            .h(px(34.))
            .flex()
            .items_center()
            .justify_between()
            .gap_2()
            .pl_1p5()
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_size(px(15.))
                    .font_weight(FontWeight::BOLD)
                    .child("Friend Activity"),
            )
            .child(hide)
    }

    fn friends_body(&self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let friends = &self.account.friends;
        if !friends.loaded {
            return match &friends.error {
                Some(error) if !friends.loading => self
                    .friends_message(error.clone(), None, palette)
                    .child(self.friends_retry(palette, cx))
                    .into_any_element(),
                _ => friends_skeleton(palette).into_any_element(),
            };
        }
        if friends.items.is_empty() {
            let note = friends
                .error
                .as_ref()
                .map(|_| self.friends_retry(palette, cx));
            return self
                .friends_message(EMPTY_TITLE.to_owned(), Some(EMPTY_TEXT), palette)
                .children(note)
                .into_any_element();
        }
        let now = now_ms();
        div()
            .id("friend-activity-list")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_0p5()
            .children(
                friends
                    .items
                    .iter()
                    .enumerate()
                    .map(|(index, friend)| self.friend_row(friend, index, now, palette, cx)),
            )
            .into_any_element()
    }

    fn friends_message(&self, title: String, text: Option<&'static str>, palette: Palette) -> Div {
        div()
            .flex()
            .flex_col()
            .items_center()
            .gap_2()
            .pt_8()
            .px_3()
            .text_center()
            .child(icon(icons::PERSON, 28., palette.muted))
            .child(
                div()
                    .text_size(px(13.5))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title),
            )
            .children(text.map(|text| {
                div()
                    .text_size(px(12.5))
                    .text_color(rgb(palette.muted))
                    .child(text)
            }))
    }

    fn friends_retry(&self, palette: Palette, cx: &mut Context<Self>) -> Stateful<Div> {
        div()
            .id(RETRY_KEY)
            .text_size(px(12.5))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(rgb(palette.accent))
            .cursor_pointer()
            .hover(|retry| retry.underline())
            .child("Try Again")
            .on_click(cx.listener(|this, _, _, cx| {
                this.refresh_friends();
                cx.notify();
            }))
    }

    fn friend_row(
        &self,
        friend: &Friend,
        index: usize,
        now: i64,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let row = (ROW, index);
        let user = friend.user.uri.clone();
        let profile = Page::Profile(user.clone());
        let track = friend.track.clone();
        let context = friend.context_uri.clone();
        let song_page = track
            .album_uri
            .as_deref()
            .and_then(page_for)
            .or_else(|| page_for(&track.uri));
        let artist_page = track.artist_uri.as_deref().and_then(page_for);
        let playing = friend.playing_now(now);
        let when = if playing {
            icon(icons::SPEAKER, 13., palette.accent).into_any_element()
        } else {
            div()
                .flex_shrink_0()
                .text_size(px(11.5))
                .text_color(rgb(palette.muted))
                .child(ago(friend.timestamp_ms, now))
                .into_any_element()
        };
        let name = div()
            .id(("friend-name", index))
            .min_w_0()
            .truncate()
            .text_size(px(13.))
            .font_weight(FontWeight::SEMIBOLD)
            .cursor_pointer()
            .hover(|name| name.underline())
            .child(friend.user.name.clone())
            .on_click({
                let profile = profile.clone();
                cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.navigate(profile.clone());
                    cx.notify();
                })
            });
        let song = div()
            .min_w_0()
            .flex()
            .items_center()
            .gap_1()
            .on_mouse_down(MouseButton::Right, {
                let (track, context) = (track.clone(), context.clone());
                self.on_menu(
                    Some(row),
                    move |_| Some(Target::track(track.clone(), context.clone(), Surface::Row)),
                    cx,
                )
            })
            .child(link(
                ("friend-song", index),
                track.title.clone(),
                song_page,
                12.5,
                palette,
                cx,
            ))
            .when(!track.artist.is_empty(), |line| {
                line.child(
                    div()
                        .flex_shrink_0()
                        .text_size(px(12.5))
                        .text_color(rgb(palette.muted))
                        .child("•"),
                )
                .child(link(
                    ("friend-artist", index),
                    track.artist.clone(),
                    artist_page,
                    12.5,
                    palette,
                    cx,
                ))
            });
        let context_line = friend.context_name.clone().map(|context_name| {
            let page = friend.context_uri.as_deref().and_then(page_for);
            div()
                .min_w_0()
                .flex()
                .items_center()
                .gap_1()
                .child(icon(icons::NOTE, 11., palette.muted))
                .child(link(
                    ("friend-context", index),
                    context_name,
                    page,
                    11.5,
                    palette,
                    cx,
                ))
        });
        div()
            .id(row)
            .flex()
            .items_start()
            .gap_2p5()
            .p(px(6.))
            .rounded(px(ROW_RADIUS))
            .when(self.menu_row(row), |item| item.bg(rgba(palette.press)))
            .hover(move |item| item.bg(rgba(palette.press)))
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| {
                this.navigate(profile.clone());
                cx.notify();
            }))
            .on_mouse_down(
                MouseButton::Right,
                self.on_menu(
                    Some(row),
                    move |_| Some(Target::User { uri: user.clone() }),
                    cx,
                ),
            )
            .child(self.avatar(
                &friend.user.name,
                friend.user.image.as_deref(),
                px(40.),
                palette,
            ))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap_0p5()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_2()
                            .child(name)
                            .child(when),
                    )
                    .child(song)
                    .children(context_line),
            )
    }
}

fn friends_skeleton(palette: Palette) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_3()
        .px_1p5()
        .children((0..4).map(move |index| {
            let key = "friend-skeleton";
            div()
                .flex()
                .items_center()
                .gap_2p5()
                .child(bone(key, index * 4, palette).size(px(40.)).rounded_full())
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .flex_col()
                        .gap_1p5()
                        .child(
                            bone(key, index * 4 + 1, palette)
                                .w(px(110.))
                                .h(px(11.))
                                .rounded(px(4.)),
                        )
                        .child(
                            bone(key, index * 4 + 2, palette)
                                .w(px(160.))
                                .h(px(10.))
                                .rounded(px(4.)),
                        )
                        .child(
                            bone(key, index * 4 + 3, palette)
                                .w(px(90.))
                                .h(px(9.))
                                .rounded(px(4.)),
                        ),
                )
        }))
}
