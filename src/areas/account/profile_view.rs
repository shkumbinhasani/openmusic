use gpui::{
    AnyElement, Context, Div, FontWeight, MouseButton, Pixels, SharedString, Stateful, div,
    prelude::*, px, rgb,
};
use spotify_client::{
    entity::{Card, Kind},
    social::{PlaylistRef, ProfileRef, RefKind, username},
};

use super::{
    profile::{BLOCKED_NOTE, FollowFilter, ProfileList},
    state::ProfileListKind,
    worker::{Listing, ProfileView},
};
use crate::{
    app::{OpenMusic, Page, page_for},
    ui::{
        Palette,
        cards::card,
        components::{bone, link, scroll_sentinel, skeleton_header, skeleton_rows},
        menu::Target,
        tracks::RowOptions,
    },
};

pub const MORE_PROFILE: &str = "more-profile";
pub const FOLLOW_PROFILE: &str = "profile-follow";
pub const UNBLOCK_PROFILE: &str = "profile-unblock";
const CARD_TEXT: f32 = 50.;
const SHELF_LIMIT: usize = 12;

pub fn playlist_card(playlist: &PlaylistRef) -> Card {
    let subtitle = if playlist.owner.is_empty() {
        "Playlist".to_owned()
    } else {
        format!("By {}", playlist.owner)
    };
    card(
        &playlist.uri,
        Kind::Playlist,
        &playlist.name,
        subtitle,
        playlist.image.clone(),
    )
}

pub fn count_label(count: u64, one: &str, many: &str) -> String {
    let digits = count.to_string();
    let mut grouped = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    format!("{grouped} {}", if count == 1 { one } else { many })
}

fn fallback_name(uri: &str) -> String {
    username(uri).unwrap_or_else(|| uri.to_owned())
}

fn heading(title: impl Into<SharedString>, note: Option<&'static str>, palette: Palette) -> Div {
    div()
        .min_w_0()
        .flex()
        .flex_col()
        .child(
            div()
                .truncate()
                .text_size(px(19.))
                .font_weight(FontWeight::BOLD)
                .child(title.into()),
        )
        .when_some(note, |heading, note| {
            heading.child(
                div()
                    .text_size(px(12.))
                    .text_color(rgb(palette.muted))
                    .child(note),
            )
        })
}

impl OpenMusic {
    fn section_heading(
        &self,
        title: &'static str,
        note: Option<&'static str>,
        show_all: Option<Page>,
        id: &'static str,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Div {
        let title = div()
            .id(id)
            .min_w_0()
            .child(heading(title, note, palette))
            .when_some(show_all.clone(), |title, page| {
                title
                    .cursor_pointer()
                    .hover(|title| title.underline())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.navigate(page.clone());
                        cx.notify();
                    }))
            });
        div()
            .flex()
            .items_end()
            .justify_between()
            .gap_4()
            .child(title)
            .when_some(show_all, |row, page| {
                row.child(div().flex_shrink_0().child(link(
                    (id, usize::MAX),
                    "Show all".to_owned(),
                    Some(page),
                    13.,
                    palette,
                    cx,
                )))
            })
    }

    pub fn people_grid(
        &self,
        people: &[ProfileRef],
        prefix: &'static str,
        card_side: Pixels,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Div {
        div()
            .flex()
            .flex_wrap()
            .gap_4()
            .children(people.iter().enumerate().map(|(index, person)| {
                let page = page_for(&person.uri);
                let target = Target::for_uri(&person.uri);
                let subtitle = match person.kind() {
                    RefKind::Artist => "Artist",
                    RefKind::User | RefKind::Other => "Profile",
                };
                div()
                    .id((prefix, index))
                    .w(card_side)
                    .flex()
                    .flex_col()
                    .gap_2()
                    .cursor_pointer()
                    .child(
                        self.avatar(&person.name, person.image.as_deref(), card_side, palette)
                            .shadow_md(),
                    )
                    .child(
                        div()
                            .w_full()
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .truncate()
                                    .child(person.name.clone()),
                            )
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(rgb(palette.muted))
                                    .truncate()
                                    .child(subtitle),
                            ),
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
    fn people_shelf(
        &self,
        title: &'static str,
        note: Option<&'static str>,
        show_all: Option<Page>,
        people: &[ProfileRef],
        prefix: &'static str,
        card_side: Pixels,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Div {
        let shown = &people[..people.len().min(SHELF_LIMIT)];
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(self.section_heading(title, note, show_all, prefix, palette, cx))
            .child(
                self.people_grid(shown, prefix, card_side, palette, cx)
                    .max_h(card_side + px(CARD_TEXT))
                    .overflow_hidden(),
            )
    }

    fn profile_header(&self, view: &ProfileView, palette: Palette, cx: &mut Context<Self>) -> Div {
        let profile = &view.profile;
        let uri = profile.uri.clone();
        let counts = profile.is_current_user || profile.show_follows;
        let mut stats: Vec<AnyElement> = Vec::new();
        if profile.total_playlists > 0 {
            stats.push(
                link(
                    ("profile-stat", 0usize),
                    count_label(
                        profile.total_playlists,
                        "Public Playlist",
                        "Public Playlists",
                    ),
                    Some(Page::ProfileList(uri.clone(), ProfileListKind::Playlists)),
                    13.5,
                    palette,
                    cx,
                )
                .into_any_element(),
            );
        }
        if counts {
            stats.push(
                link(
                    ("profile-stat", 1usize),
                    count_label(profile.followers, "Follower", "Followers"),
                    Some(Page::ProfileList(uri.clone(), ProfileListKind::Followers)),
                    13.5,
                    palette,
                    cx,
                )
                .into_any_element(),
            );
            stats.push(
                link(
                    ("profile-stat", 2usize),
                    count_label(profile.following, "Following", "Following"),
                    Some(Page::ProfileList(uri.clone(), ProfileListKind::Following)),
                    13.5,
                    palette,
                    cx,
                )
                .into_any_element(),
            );
        }
        let count = stats.len();
        let stats = stats
            .into_iter()
            .enumerate()
            .flat_map(|(index, stat)| {
                let dot = (index + 1 < count).then(|| {
                    div()
                        .flex_shrink_0()
                        .text_size(px(13.5))
                        .text_color(rgb(palette.muted))
                        .child("•")
                        .into_any_element()
                });
                std::iter::once(stat).chain(dot)
            })
            .collect::<Vec<_>>();
        let target = Target::User { uri: uri.clone() };
        let actions = self.profile_actions(view, palette, cx);
        let blocked = !profile.is_current_user
            && self.account.profiles.known_blocking(&profile.uri) == Some(true);
        div()
            .flex()
            .items_center()
            .gap_6()
            .on_mouse_down(
                MouseButton::Right,
                self.on_menu(None, move |_| Some(target.clone()), cx),
            )
            .child(
                self.avatar(&profile.name, profile.image.as_deref(), px(176.), palette)
                    .shadow_lg(),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .items_start()
                    .gap_1p5()
                    .child(
                        div()
                            .text_size(px(11.5))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(palette.accent))
                            .child("PROFILE"),
                    )
                    .child(
                        div()
                            .text_size(px(40.))
                            .font_weight(FontWeight::BOLD)
                            .truncate()
                            .w_full()
                            .child(profile.name.clone()),
                    )
                    .when(!stats.is_empty(), |column| {
                        column.child(
                            div()
                                .w_full()
                                .min_w_0()
                                .flex()
                                .items_center()
                                .gap_1p5()
                                .children(stats),
                        )
                    })
                    .when(blocked, |column| {
                        column.child(
                            div()
                                .text_size(px(12.5))
                                .text_color(rgb(palette.muted))
                                .child(BLOCKED_NOTE),
                        )
                    })
                    .when_some(actions, |column, actions| column.child(actions)),
            )
    }

    fn profile_actions(
        &self,
        view: &ProfileView,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Option<Div> {
        let target = Target::User {
            uri: view.profile.uri.clone(),
        };
        let more = self
            .has_menu(&target)
            .then(|| self.menu_button(MORE_PROFILE, &view.profile.name, palette, cx));
        let follow = self.follow_button(view, palette, cx);
        if more.is_none() && follow.is_none() {
            return None;
        }
        Some(
            div()
                .mt_3()
                .flex()
                .items_center()
                .gap_2()
                .children(follow)
                .children(more),
        )
    }

    fn follow_button(
        &self,
        view: &ProfileView,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Option<Stateful<Div>> {
        let profile = &view.profile;
        if profile.is_current_user || self.is_own_profile(&profile.uri) {
            return None;
        }
        if self.account.profiles.known_blocking(&profile.uri) == Some(true) {
            return Some(self.text_control(UNBLOCK_PROFILE, "Unblock", false, palette, cx));
        }
        let following = self
            .account
            .profiles
            .known_following(&profile.uri)
            .unwrap_or(profile.is_following);
        if !profile.allow_follows && !following {
            return None;
        }
        let label = if following { "Following" } else { "Follow" };
        Some(self.text_control(FOLLOW_PROFILE, label, false, palette, cx))
    }

    pub fn profile_page(
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
        let view = match &self.account.profiles.profile {
            Some((loaded, Ok(view))) if loaded == uri => view,
            Some((loaded, Err(error))) if loaded == uri => {
                return page
                    .child(self.page_status(error.clone(), palette))
                    .into_any_element();
            }
            _ => {
                return page
                    .child(skeleton_header("profile-header", true, palette))
                    .child(self.skeleton_shelf("profile-skeleton", card_side, palette))
                    .into_any_element();
            }
        };
        let profile = &view.profile;
        let own = profile.is_current_user;
        let counts = own || profile.show_follows;
        let list = |kind| Some(Page::ProfileList(uri.to_owned(), kind));
        let mut sections: Vec<AnyElement> = Vec::new();
        if let Some(top) = view.top.as_ref().filter(|_| own) {
            if !top.artists.is_empty() {
                sections.push(
                    self.people_shelf(
                        "Top artists this month",
                        Some("Only visible to you"),
                        list(ProfileListKind::TopArtists),
                        &top.artists,
                        "profile-top-artist",
                        card_side,
                        palette,
                        cx,
                    )
                    .into_any_element(),
                );
            }
            if !top.tracks.is_empty() {
                let more = (top.tracks_total > top.tracks.len() as u64)
                    .then(|| Page::ProfileList(uri.to_owned(), ProfileListKind::TopTracks));
                let options = RowOptions {
                    numbered: true,
                    ..RowOptions::default()
                };
                sections.push(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(self.section_heading(
                            "Top tracks this month",
                            Some("Only visible to you"),
                            more,
                            "profile-top-tracks",
                            palette,
                            cx,
                        ))
                        .child(self.track_list(
                            &top.tracks,
                            "profile-top-track",
                            0,
                            &options,
                            palette,
                            cx,
                        ))
                        .into_any_element(),
                );
            }
        }
        if !profile.playlists.is_empty() {
            let cards: Vec<Card> = profile.playlists.iter().map(playlist_card).collect();
            sections.push(
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(self.section_heading(
                        "Public Playlists",
                        None,
                        list(ProfileListKind::Playlists),
                        "profile-playlists",
                        palette,
                        cx,
                    ))
                    .child(
                        self.card_grid(&cards, "profile-playlist", card_side, palette, cx)
                            .max_h(card_side + px(CARD_TEXT))
                            .overflow_hidden(),
                    )
                    .into_any_element(),
            );
        }
        if !profile.recent_artists.is_empty() {
            sections.push(
                self.people_shelf(
                    "Recently played artists",
                    None,
                    list(ProfileListKind::RecentArtists),
                    &profile.recent_artists,
                    "profile-recent-artist",
                    card_side,
                    palette,
                    cx,
                )
                .into_any_element(),
            );
        }
        if counts && !view.followers.is_empty() {
            sections.push(
                self.people_shelf(
                    "Followers",
                    None,
                    list(ProfileListKind::Followers),
                    &view.followers,
                    "profile-follower",
                    card_side,
                    palette,
                    cx,
                )
                .into_any_element(),
            );
        }
        if counts && !view.following.is_empty() {
            sections.push(
                self.people_shelf(
                    "Following",
                    None,
                    list(ProfileListKind::Following),
                    &view.following,
                    "profile-following",
                    card_side,
                    palette,
                    cx,
                )
                .into_any_element(),
            );
        }
        let empty = sections.is_empty();
        page.child(self.profile_header(view, palette, cx))
            .children(sections)
            .when(empty, |page| {
                page.child(self.page_status(
                    if own {
                        "Your public playlists and the people you follow show up here."
                    } else {
                        "Nothing public here yet."
                    },
                    palette,
                ))
            })
            .into_any_element()
    }

    fn skeleton_shelf(&self, key: &'static str, card_side: Pixels, palette: Palette) -> Div {
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(bone(key, 0, palette).w(px(180.)).h(px(20.)).rounded(px(5.)))
            .child(div().flex().gap_4().children((0..4).map(|index| {
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        bone(key, 1 + index * 3, palette)
                            .size(card_side)
                            .rounded_full(),
                    )
                    .child(
                        bone(key, 2 + index * 3, palette)
                            .w(card_side * 0.7)
                            .h(px(12.))
                            .rounded(px(4.)),
                    )
                    .child(
                        bone(key, 3 + index * 3, palette)
                            .w(card_side * 0.45)
                            .h(px(11.))
                            .rounded(px(4.)),
                    )
            })))
    }

    pub fn profile_list_page(
        &self,
        uri: &str,
        kind: ProfileListKind,
        card_side: Pixels,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let name = self
            .account
            .profiles
            .loaded(uri)
            .map(|view| view.profile.name.clone())
            .or_else(|| {
                self.account()
                    .filter(|account| account.uri == uri)
                    .map(|account| account.name.clone())
            })
            .unwrap_or_else(|| fallback_name(uri));
        let back_to = Page::Profile(uri.to_owned());
        let page = div()
            .flex()
            .flex_col()
            .gap_6()
            .child(div().child(self.back_button(palette, cx)))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_size(px(30.))
                            .font_weight(FontWeight::BOLD)
                            .child(kind.title()),
                    )
                    .child(div().flex().min_w_0().child(link(
                        ("profile-list-owner", 0usize),
                        name,
                        Some(back_to),
                        14.,
                        palette,
                        cx,
                    ))),
            );
        let list = self.account.profiles.list_for(uri, kind);
        let page = if kind == ProfileListKind::Following {
            let filter = self.account.profiles.filter;
            let chips: Vec<SharedString> = FollowFilter::FILTERS
                .iter()
                .map(|filter| filter.label().into())
                .collect();
            let selected = FollowFilter::FILTERS
                .iter()
                .position(|candidate| *candidate == filter);
            page.child(self.chip_row(
                "profile-filter",
                &chips,
                selected,
                |app, index, _, _| {
                    app.account.profiles.filter = FollowFilter::FILTERS[index];
                },
                palette,
                cx,
            ))
        } else {
            page
        };
        let Some(ProfileList {
            result: Some(result),
            loading_more,
            ..
        }) = list
        else {
            let skeleton = if kind == ProfileListKind::TopTracks {
                skeleton_rows("profile-list-rows", 8, true, palette)
            } else {
                self.skeleton_shelf("profile-list-skeleton", card_side, palette)
            };
            return page.child(skeleton).into_any_element();
        };
        let listing = match result {
            Ok(listing) => listing,
            Err(error) => {
                return page
                    .child(self.page_status(error.clone(), palette))
                    .into_any_element();
            }
        };
        let body = match listing {
            Listing::People(people) => {
                let filter = if kind == ProfileListKind::Following {
                    self.account.profiles.filter
                } else {
                    FollowFilter::All
                };
                let shown: Vec<ProfileRef> = people
                    .iter()
                    .filter(|person| filter.keeps(person))
                    .cloned()
                    .collect();
                if shown.is_empty() {
                    self.page_status(empty_message(kind, filter), palette)
                } else {
                    self.people_grid(&shown, "profile-list-person", card_side, palette, cx)
                }
            }
            Listing::Playlists { items, total } => {
                if items.is_empty() {
                    self.page_status(empty_message(kind, FollowFilter::All), palette)
                } else {
                    let cards: Vec<Card> = items.iter().map(playlist_card).collect();
                    let more = (items.len() as u64) < *total;
                    div()
                        .flex()
                        .flex_col()
                        .gap_4()
                        .child(self.card_grid(
                            &cards,
                            "profile-list-playlist",
                            card_side,
                            palette,
                            cx,
                        ))
                        .when(more, |column| {
                            column.child(scroll_sentinel(&self.account.profiles.sentinel))
                        })
                        .when(*loading_more, |column| {
                            column.child(self.skeleton_shelf(
                                "profile-list-more",
                                card_side,
                                palette,
                            ))
                        })
                }
            }
            Listing::Tracks(tracks) => {
                if tracks.is_empty() {
                    self.page_status(empty_message(kind, FollowFilter::All), palette)
                } else {
                    let options = RowOptions {
                        numbered: true,
                        ..RowOptions::default()
                    };
                    self.track_list(tracks, "profile-list-track", 0, &options, palette, cx)
                }
            }
        };
        page.child(body).into_any_element()
    }
}

pub fn empty_message(kind: ProfileListKind, filter: FollowFilter) -> &'static str {
    match (kind, filter) {
        (ProfileListKind::Followers, _) => "No followers yet.",
        (ProfileListKind::Following, FollowFilter::Artists) => "No artists here yet.",
        (ProfileListKind::Following, FollowFilter::Profiles) => "No profiles here yet.",
        (ProfileListKind::Following, FollowFilter::All) => "Not following anyone yet.",
        (ProfileListKind::Playlists, _) => "No public playlists yet.",
        (ProfileListKind::RecentArtists, _) => "No recently played artists to show.",
        (ProfileListKind::TopArtists, _) => "Listen to more music to see your top artists.",
        (ProfileListKind::TopTracks, _) => "Listen to more music to see your top tracks.",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_counts() {
        assert_eq!(count_label(0, "Follower", "Followers"), "0 Followers");
        assert_eq!(count_label(1, "Follower", "Followers"), "1 Follower");
        assert_eq!(count_label(999, "Follower", "Followers"), "999 Followers");
        assert_eq!(
            count_label(1000, "Follower", "Followers"),
            "1,000 Followers"
        );
        assert_eq!(
            count_label(246_741, "Follower", "Followers"),
            "246,741 Followers"
        );
        assert_eq!(
            count_label(1_234_567, "Following", "Following"),
            "1,234,567 Following"
        );
    }

    #[test]
    fn builds_playlist_cards() {
        let mix = playlist_card(&PlaylistRef {
            uri: "spotify:playlist:0000000000000000000001".into(),
            name: "Mix".into(),
            owner: "Someone".into(),
            ..PlaylistRef::default()
        });
        assert_eq!(mix.subtitle, "By Someone");
        let bare = playlist_card(&PlaylistRef::default());
        assert_eq!(bare.subtitle, "Playlist");
        assert_eq!(fallback_name("spotify:user:a%20b"), "a b");
    }

    #[test]
    fn explains_empty_lists() {
        assert_eq!(
            empty_message(ProfileListKind::Following, FollowFilter::Artists),
            "No artists here yet."
        );
        for kind in ProfileListKind::KINDS {
            assert!(!empty_message(kind, FollowFilter::All).is_empty());
        }
    }
}
