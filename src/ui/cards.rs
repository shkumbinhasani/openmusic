use std::rc::Rc;

use gpui::{
    Context, Div, FontWeight, MouseButton, ObjectFit, Pixels, SharedString, Stateful, Window, div,
    img, prelude::*, px, rgb, rgba,
};
use spotify_client::{
    Track,
    entity::{Card, Kind},
};

use super::{
    Palette,
    components::{artwork, explicit_badge, link},
    menu::{Surface, Target},
};
use crate::{
    app::{OpenMusic, Page, page_for},
    overlay::Label,
    worker::trusted_image,
};

const CARD_TEXT_HEIGHT: f32 = 50.;

#[derive(Clone, Debug, PartialEq)]
pub struct Face {
    pub name: String,
    pub image: Option<String>,
}

pub fn card(
    uri: impl Into<String>,
    kind: Kind,
    title: impl Into<String>,
    subtitle: impl Into<String>,
    image: Option<String>,
) -> Card {
    Card {
        uri: uri.into(),
        kind,
        title: title.into(),
        subtitle: subtitle.into(),
        image,
        format: None,
        color: None,
        reason: None,
        explicit: false,
        playable: true,
    }
}

pub fn round_kind(kind: Kind) -> bool {
    matches!(kind, Kind::Artist | Kind::User | Kind::Author)
}

pub fn kind_label(kind: Kind) -> &'static str {
    match kind {
        Kind::Track => "Song",
        Kind::Album => "Album",
        Kind::Artist => "Artist",
        Kind::Playlist | Kind::LikedSongs => "Playlist",
        Kind::Show => "Podcast",
        Kind::Episode => "Episode",
        Kind::Audiobook => "Audiobook",
        Kind::Author => "Author",
        Kind::User => "Profile",
        Kind::Genre => "Genre",
        Kind::Feature => "",
    }
}

pub fn hero_subtitle(card: &Card) -> String {
    let label = kind_label(card.kind);
    if card.subtitle.is_empty() || card.subtitle == label {
        label.to_owned()
    } else if label.is_empty() {
        card.subtitle.clone()
    } else {
        format!("{label} • {}", card.subtitle)
    }
}

pub fn card_target(card: &Card) -> Option<Target> {
    if card.kind != Kind::Track {
        return Target::for_uri(&card.uri);
    }
    let track = Track {
        uri: card.uri.clone(),
        title: card.title.clone(),
        artist: card.subtitle.clone(),
        cover: card.image.clone(),
        explicit: card.explicit,
        ..Track::default()
    };
    Some(Target::track(track, None, Surface::Card))
}

fn initial(name: &str) -> String {
    name.chars()
        .find(|letter| letter.is_alphanumeric())
        .map(|letter| letter.to_uppercase().collect())
        .unwrap_or_else(|| "?".to_owned())
}

fn tint(name: &str) -> u32 {
    const TINTS: [u32; 6] = [0x8e6cdb, 0x3f8fd6, 0x2fa37a, 0xd08a2b, 0xd0566c, 0x6b7a8f];
    let sum = name.bytes().fold(0usize, |sum, byte| {
        sum.wrapping_mul(31).wrapping_add(usize::from(byte))
    });
    TINTS[sum % TINTS.len()]
}

impl OpenMusic {
    pub fn avatar(&self, name: &str, image: Option<&str>, side: Pixels, palette: Palette) -> Div {
        let loaded = image
            .filter(|url| trusted_image(url))
            .and_then(|url| self.covers.get(url));
        let circle = div()
            .size(side)
            .flex_shrink_0()
            .rounded_full()
            .overflow_hidden()
            .flex()
            .items_center()
            .justify_center();
        match loaded {
            Some(image) => circle.bg(rgb(palette.raised)).child(
                img(image.clone())
                    .size_full()
                    .object_fit(ObjectFit::Cover)
                    .rounded_full(),
            ),
            None => circle
                .bg(rgb(tint(name)))
                .text_color(rgb(0xffffff))
                .text_size(side * 0.42)
                .font_weight(FontWeight::SEMIBOLD)
                .child(initial(name)),
        }
    }

    pub fn facepile(&self, faces: &[Face], side: Pixels, max: usize, palette: Palette) -> Div {
        let shown = faces.len().min(max.max(1));
        let hidden = faces.len() - shown;
        let ring = |face: Div, index: usize| {
            div()
                .flex_shrink_0()
                .rounded_full()
                .border_2()
                .border_color(rgb(palette.canvas))
                .when(index > 0, |face| face.ml(side * -0.3))
                .child(face)
        };
        div()
            .flex()
            .items_center()
            .children(faces.iter().take(shown).enumerate().map(|(index, face)| {
                ring(
                    self.avatar(&face.name, face.image.as_deref(), side, palette),
                    index,
                )
            }))
            .when(hidden > 0, |pile| {
                pile.child(ring(
                    div()
                        .size(side)
                        .rounded_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(rgb(palette.raised))
                        .text_color(rgb(palette.muted))
                        .text_size(side * 0.36)
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(format!("+{hidden}")),
                    shown,
                ))
            })
    }

    pub fn card_grid(
        &self,
        cards: &[Card],
        prefix: &'static str,
        card_side: Pixels,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Div {
        div()
            .flex()
            .flex_wrap()
            .gap_4()
            .children(cards.iter().enumerate().map(|(index, card)| {
                let image = card.image.as_ref().and_then(|url| self.covers.get(url));
                let radius = if round_kind(card.kind) {
                    card_side / 2.
                } else {
                    px(12.)
                };
                let page = page_for(&card.uri);
                let target = card_target(card);
                let subtitle = div()
                    .text_size(px(12.))
                    .text_color(rgb(palette.muted))
                    .truncate()
                    .child(card.subtitle.clone());
                div()
                    .id((prefix, index))
                    .w(card_side)
                    .flex()
                    .flex_col()
                    .gap_2()
                    .cursor_pointer()
                    .when(!card.playable, |card| card.opacity(0.5))
                    .child(artwork(image, card_side, radius, palette).shadow_md())
                    .child(
                        div()
                            .w_full()
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .truncate()
                                    .child(card.title.clone()),
                            )
                            .map(|text| {
                                if card.explicit {
                                    text.child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap_1()
                                            .min_w_0()
                                            .child(explicit_badge(palette))
                                            .child(subtitle.min_w_0()),
                                    )
                                } else {
                                    text.child(subtitle)
                                }
                            }),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(page) = page.clone() {
                            this.navigate(page);
                            cx.notify();
                        }
                    }))
                    .on_mouse_down(
                        MouseButton::Right,
                        self.on_menu(None, move |_| target.clone(), cx),
                    )
            }))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn shelf(
        &self,
        title: impl Into<SharedString>,
        show_all: Option<Page>,
        cards: &[Card],
        prefix: &'static str,
        card_side: Pixels,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Div {
        let title = title.into();
        let heading = div()
            .id((prefix, usize::MAX))
            .min_w_0()
            .truncate()
            .text_size(px(19.))
            .font_weight(FontWeight::BOLD)
            .child(title);
        let heading = match show_all.clone() {
            Some(page) => heading
                .cursor_pointer()
                .hover(|heading| heading.underline())
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.navigate(page.clone());
                    cx.notify();
                })),
            None => heading,
        };
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_4()
                    .child(heading)
                    .when_some(show_all, |row, page| {
                        row.child(div().flex_shrink_0().child(link(
                            (prefix, usize::MAX - 1),
                            "Show all".to_owned(),
                            Some(page),
                            13.,
                            palette,
                            cx,
                        )))
                    }),
            )
            .child(
                self.card_grid(cards, prefix, card_side, palette, cx)
                    .max_h(card_side + px(CARD_TEXT_HEIGHT))
                    .overflow_hidden(),
            )
    }

    pub fn hero_card(
        &self,
        card: &Card,
        prefix: &'static str,
        play: Option<&'static str>,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let image = card.image.as_ref().and_then(|url| self.covers.get(url));
        let side = px(92.);
        let radius = if round_kind(card.kind) {
            side / 2.
        } else {
            px(10.)
        };
        let page = page_for(&card.uri);
        let target = card_target(card);
        div()
            .id(prefix)
            .flex()
            .items_center()
            .gap_5()
            .p_4()
            .rounded(px(16.))
            .bg(rgba(palette.press))
            .cursor_pointer()
            .when(!card.playable, |hero| hero.opacity(0.5))
            .on_click(cx.listener(move |this, _, _, cx| {
                if let Some(page) = page.clone() {
                    this.navigate(page);
                    cx.notify();
                }
            }))
            .on_mouse_down(
                MouseButton::Right,
                self.on_menu(None, move |_| target.clone(), cx),
            )
            .child(artwork(image, side, radius, palette).shadow_md())
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_size(px(28.))
                            .font_weight(FontWeight::BOLD)
                            .truncate()
                            .child(card.title.clone()),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1p5()
                            .min_w_0()
                            .when(card.explicit, |line| line.child(explicit_badge(palette)))
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(13.5))
                                    .text_color(rgb(palette.muted))
                                    .child(hero_subtitle(card)),
                            ),
                    ),
            )
            .when_some(play, |hero, key| {
                hero.child(
                    self.control(key, Label::Symbol("play.fill"), true, palette, cx)
                        .size(px(48.)),
                )
            })
    }

    pub fn chip_row(
        &self,
        key: &'static str,
        chips: &[SharedString],
        selected: Option<usize>,
        pick: impl Fn(&mut OpenMusic, usize, &mut Window, &mut Context<OpenMusic>) + 'static,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Div {
        let pick = Rc::new(pick);
        div()
            .flex()
            .flex_wrap()
            .gap_2()
            .children(chips.iter().enumerate().map(|(index, label)| {
                let chosen = selected == Some(index);
                let pick = pick.clone();
                let chip = if chosen {
                    div()
                        .rounded(px(16.))
                        .bg(rgb(palette.accent))
                        .text_color(rgb(0xffffff))
                } else {
                    palette.glass(format!("{key}-{index}"), 16.)
                };
                chip.id((key, index))
                    .h(px(32.))
                    .flex()
                    .items_center()
                    .px_3p5()
                    .flex_shrink_0()
                    .cursor_pointer()
                    .text_size(px(13.))
                    .font_weight(FontWeight::MEDIUM)
                    .when(!chosen, |chip| chip.hover(|chip| chip.opacity(0.8)))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        pick(this, index, window, cx);
                        cx.notify();
                    }))
                    .child(label.clone())
            }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(kind: Kind, subtitle: &str) -> Card {
        card("spotify:album:x", kind, "Title", subtitle, None)
    }

    #[test]
    fn hero_subtitles_name_the_kind_once() {
        assert_eq!(
            hero_subtitle(&sample(Kind::Track, "Radiohead")),
            "Song • Radiohead"
        );
        assert_eq!(hero_subtitle(&sample(Kind::Artist, "Artist")), "Artist");
        assert_eq!(hero_subtitle(&sample(Kind::User, "Profile")), "Profile");
        assert_eq!(hero_subtitle(&sample(Kind::Genre, "")), "Genre");
        assert_eq!(
            hero_subtitle(&sample(Kind::Feature, "Made for you")),
            "Made for you"
        );
    }

    #[test]
    fn avatars_fall_back_to_a_capital_initial() {
        assert_eq!(initial("ada lovelace"), "A");
        assert_eq!(initial("  @émile"), "É");
        assert_eq!(initial(""), "?");
        assert_eq!(tint("same"), tint("same"));
    }

    #[test]
    fn song_cards_target_a_track_menu() {
        let mut song = sample(Kind::Track, "Radiohead");
        song.uri = "spotify:track:t".into();
        let Some(Target::Track(target)) = card_target(&song) else {
            panic!("song cards open a track menu");
        };
        assert_eq!(target.track.uri, "spotify:track:t");
        assert_eq!(target.surface, Surface::Card);
        assert_eq!(
            card_target(&sample(Kind::Album, "")),
            Some(Target::Album {
                uri: "spotify:album:x".into()
            })
        );
    }
}
