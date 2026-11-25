use gpui::{AnyElement, Context, FontWeight, MouseButton, Pixels, div, hsla, prelude::*, px, rgb};
use spotify_client::{Track, catalog::Collection, entity, playlist, playlist::Level};

use super::state::{PlaylistInfo, owner_label};
use crate::{
    app::{OpenMusic, Page},
    icons,
    overlay::Label,
    ui::{
        Palette,
        cards::Face,
        components::{artwork, link, scroll_sentinel, skeleton_header, skeleton_rows},
        dock::DockItem,
        menu::{Group, MenuItem, Row, Target},
        sidebar::NavEntry,
        tracks::RowOptions,
    },
};

pub fn songs_label(count: usize) -> String {
    if count == 1 {
        "1 song".to_owned()
    } else {
        format!("{count} songs")
    }
}

pub fn kind_label(collection: &Collection, info: Option<&PlaylistInfo>) -> &'static str {
    if collection.kind != "PLAYLIST" {
        return "Album";
    }
    match info {
        Some(info) if info.base == Some(Level::Blocked) => "Private Playlist",
        Some(info) if info.owned && info.base.is_some() => "Public Playlist",
        _ => "Playlist",
    }
}

pub fn summary(collection: &Collection) -> String {
    let count = songs_label(collection.total);
    if collection.next.is_some() {
        return count;
    }
    let total: u64 = collection
        .tracks
        .iter()
        .map(|track| u64::from(track.duration_ms))
        .sum();
    match entity::long_duration(total.min(u64::from(u32::MAX)) as u32) {
        Some(duration) => format!("{count}, {duration}"),
        None => count,
    }
}

impl OpenMusic {
    pub fn collection_page(
        &self,
        uri: &str,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let page = div()
            .flex()
            .flex_col()
            .gap_7()
            .child(div().child(self.back_button(palette, cx)));
        let Some((loaded, result)) = self
            .playlists
            .collection
            .as_ref()
            .filter(|(loaded, _)| loaded == uri)
        else {
            let numbered = uri.starts_with("spotify:album:");
            return page
                .child(skeleton_header("collection-header", false, palette))
                .child(skeleton_rows("collection-rows", 8, numbered, palette));
        };
        let collection = match result {
            Ok(collection) => collection,
            Err(error) => return page.child(self.page_status(error.clone(), palette)),
        };
        let is_playlist = playlist::playlist_id(loaded).is_some();
        let info = if is_playlist {
            self.playlist_info(loaded)
        } else {
            None
        };
        let editable = info
            .as_ref()
            .is_some_and(|info| info.capabilities.edit_metadata);
        let image = collection
            .cover
            .as_ref()
            .and_then(|url| self.covers.get(url));
        let has_menu = Target::for_uri(loaded).is_some_and(|target| self.has_menu(&target));
        let header_uri = loaded.clone();
        let album = collection.kind == "ALBUM";
        let (cover_uri, title_uri) = (loaded.clone(), loaded.clone());

        let cover = div()
            .id("collection-cover")
            .group("collection-cover")
            .relative()
            .flex_shrink_0()
            .child(artwork(image, px(184.), px(16.), palette).shadow_lg())
            .when(editable, |cover| {
                cover
                    .cursor_pointer()
                    .child(
                        div()
                            .absolute()
                            .inset_0()
                            .rounded(px(16.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(hsla(0., 0., 0., 0.5))
                            .text_color(rgb(0xffffff))
                            .text_size(px(13.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .opacity(0.)
                            .group_hover("collection-cover", |style| style.opacity(1.))
                            .child("Choose photo"),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.edit_details(&cover_uri, window, cx);
                        cx.notify();
                    }))
            });

        let title = div()
            .id("collection-title")
            .text_size(px(34.))
            .font_weight(FontWeight::BOLD)
            .truncate()
            .w_full()
            .child(collection.name.clone())
            .when(editable, |title| {
                title
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.edit_details(&title_uri, window, cx);
                        cx.notify();
                    }))
            });

        let meta = if is_playlist {
            let owner = collection.owner.clone();
            let owner_name = info.as_ref().and_then(|info| info.owner_name.clone());
            div()
                .w_full()
                .flex()
                .items_center()
                .gap_1()
                .min_w_0()
                .when_some(owner, |line, owner| {
                    line.child(link(
                        ("collection-owner", 0usize),
                        owner_label(&owner, owner_name.as_deref()),
                        Some(Page::Profile(format!("spotify:user:{owner}"))),
                        13.5,
                        palette,
                        cx,
                    ))
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_size(px(13.5))
                            .text_color(rgb(palette.muted))
                            .child("•"),
                    )
                })
                .child(
                    div()
                        .flex_shrink_0()
                        .text_size(px(13.5))
                        .text_color(rgb(palette.muted))
                        .child(summary(collection)),
                )
        } else {
            div().w_full().flex().min_w_0().child(link(
                ("collection-subtitle", 0usize),
                collection.subtitle.clone(),
                collection.artist_uri.clone().map(Page::Artist),
                13.5,
                palette,
                cx,
            ))
        };

        let faces: Vec<Face> = playlist::canonical(loaded)
            .and_then(|uri| self.playlists.permissions.get(&uri))
            .filter(|permissions| permissions.members.len() > 1)
            .map(|permissions| {
                permissions
                    .members
                    .iter()
                    .map(|member| Face {
                        name: member.name.clone(),
                        image: member.image.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        let members_uri = loaded.clone();

        let follow = info
            .as_ref()
            .filter(|info| !info.owned)
            .and_then(|info| info.saved);
        let actions = div()
            .mt_3()
            .flex()
            .items_center()
            .gap_2()
            .child(
                self.text_control("play-collection", "Play", true, palette, cx)
                    .w(px(96.)),
            )
            .when(!collection.tracks.is_empty(), |actions| {
                actions.child(
                    self.control(
                        "shuffle-collection",
                        Label::Symbol("shuffle"),
                        false,
                        palette,
                        cx,
                    )
                    .size(px(36.)),
                )
            })
            .when_some(follow, |actions, saved| {
                actions.child(
                    self.control(
                        "follow-collection",
                        Label::Symbol(if saved {
                            "checkmark.circle.fill"
                        } else {
                            "plus.circle"
                        }),
                        false,
                        palette,
                        cx,
                    )
                    .size(px(36.)),
                )
            })
            .when(has_menu, |actions| {
                actions.child(self.menu_button("more-collection", &collection.name, palette, cx))
            });

        let header = div()
            .flex()
            .items_center()
            .gap_6()
            .on_mouse_down(
                MouseButton::Right,
                self.on_menu(None, move |_| Target::for_uri(&header_uri), cx),
            )
            .child(cover)
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
                            .child(kind_label(collection, info.as_ref())),
                    )
                    .child(title)
                    .when(
                        is_playlist && !collection.description.is_empty(),
                        |column| {
                            column.child(
                                div()
                                    .w_full()
                                    .text_size(px(13.))
                                    .text_color(rgb(palette.muted))
                                    .line_clamp(2)
                                    .child(collection.description.clone()),
                            )
                        },
                    )
                    .child(meta)
                    .when(!faces.is_empty(), |column| {
                        column.child(
                            div()
                                .id("collection-members")
                                .cursor_pointer()
                                .child(self.facepile(&faces, px(24.), 5, palette))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.show_collaborators(&members_uri, window, cx);
                                    cx.notify();
                                })),
                        )
                    })
                    .child(actions),
            );

        let rows = if collection.tracks.is_empty() && collection.next.is_none() {
            self.page_status(
                if is_playlist {
                    "This playlist is empty."
                } else {
                    "This is empty."
                },
                palette,
            )
        } else {
            let options = RowOptions {
                numbered: album,
                context: Some(loaded.clone()),
                show_album: !album,
                show_added: is_playlist,
                ..RowOptions::default()
            };
            self.track_list(
                &collection.tracks,
                "collection-track",
                0,
                &options,
                palette,
                cx,
            )
            .when(collection.next.is_some(), |list| {
                list.child(scroll_sentinel(&self.playlists.sentinel))
                    .child(skeleton_rows("collection-more", 3, album, palette))
            })
        };
        page.child(header).child(rows)
    }
}

impl OpenMusic {
    pub fn blend_invitation_page(
        &self,
        _id: &str,
        _card_side: Pixels,
        palette: Palette,
        _cx: &mut Context<Self>,
    ) -> AnyElement {
        self.page_status("Coming soon", palette).into_any_element()
    }
}

pub fn playlist_menu(info: &PlaylistInfo) -> Vec<(Group, MenuItem)> {
    let mut items = Vec::new();
    let saved = info.saved.unwrap_or(false);
    let uri = info.uri.clone();
    let deletable = info.owned || info.capabilities.delete;
    if !deletable && let Some(saved) = info.saved {
        let uri = uri.clone();
        items.push((
            Group::Library,
            if saved {
                MenuItem::action("Remove from Your Library", move |app, _, _| {
                    app.follow_playlist(&uri, false)
                })
            } else {
                MenuItem::action("Add to Your Library", move |app, _, _| {
                    app.follow_playlist(&uri, true)
                })
            },
        ));
    }
    let mut edit = Vec::new();
    if saved && info.capabilities.publish && info.base != Some(Level::Blocked) {
        let uri = uri.clone();
        let public = info.on_profile;
        edit.push(MenuItem::action(
            if public {
                "Remove from profile"
            } else {
                "Add to profile"
            },
            move |app, _, _| app.set_playlist_on_profile(&uri, !public),
        ));
    }
    if info.capabilities.edit_metadata {
        let uri = uri.clone();
        edit.push(MenuItem::action("Edit details", move |app, window, cx| {
            app.edit_details(&uri, window, cx)
        }));
    }
    if saved && deletable {
        let uri = uri.clone();
        edit.push(
            MenuItem::action("Delete", move |app, window, cx| {
                app.delete_playlist(&uri, window, cx)
            })
            .key("backspace"),
        );
    }
    let mut sharing = Vec::new();
    if info.capabilities.administrate {
        let privacy = {
            let uri = uri.clone();
            match info.base {
                Some(Level::Blocked) => MenuItem::action("Make public", move |app, window, cx| {
                    app.set_playlist_private(&uri, false, window, cx)
                }),
                Some(_) => MenuItem::action("Make private", move |app, window, cx| {
                    app.set_playlist_private(&uri, true, window, cx)
                }),
                None => MenuItem::action("Make private", |_, _, _| {}).enabled(false),
            }
        };
        sharing.push(privacy);
        let uri = uri.clone();
        sharing.push(MenuItem::action(
            "Invite collaborators",
            move |app, _, _| app.invite_collaborators(uri.clone()),
        ));
    }
    if info.capabilities.cancel_membership {
        let uri = uri.clone();
        sharing.push(MenuItem::action(
            "Leave playlist",
            move |app, window, cx| app.leave_playlist(uri.clone(), window, cx),
        ));
    }
    if info.members > 1 {
        sharing.push(MenuItem::action("Collaborators", move |app, window, cx| {
            app.show_collaborators(&uri, window, cx)
        }));
    }
    if !sharing.is_empty() {
        if !edit.is_empty() || !items.is_empty() {
            edit.push(MenuItem::Separator);
        }
        edit.extend(sharing);
    }
    items.extend(edit.into_iter().map(|item| (Group::Edit, item)));
    items
}

pub fn menu_items(target: &Target, app: &OpenMusic) -> Vec<(Group, MenuItem)> {
    let Target::Playlist { uri } = target else {
        return Vec::new();
    };
    app.playlist_info(uri)
        .map(|info| playlist_menu(&info))
        .unwrap_or_default()
}

pub fn nav_entries(app: &OpenMusic) -> Vec<NavEntry> {
    if app.playlists.user.is_none() {
        return Vec::new();
    }
    vec![NavEntry::new(
        "nav-new-playlist",
        590,
        icons::PLUS,
        "New Playlist",
        false,
        |app, _, _| app.create_playlist(None, None, Vec::new(), true),
    )]
}

pub fn dock_items(
    _app: &OpenMusic,
    _roomy: bool,
    _palette: Palette,
    _cx: &mut Context<OpenMusic>,
) -> Vec<DockItem> {
    Vec::new()
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
    use spotify_client::catalog::Capabilities;

    use super::*;
    use crate::ui::menu::grouped;

    fn titles(items: Vec<(Group, MenuItem)>) -> Vec<String> {
        grouped(items)
            .into_iter()
            .map(|item| match item {
                MenuItem::Action { title, enabled, .. } => {
                    if enabled {
                        title
                    } else {
                        format!("{title} [x]")
                    }
                }
                MenuItem::Separator => "──".to_owned(),
                _ => "?".to_owned(),
            })
            .collect()
    }

    fn owned() -> PlaylistInfo {
        PlaylistInfo {
            uri: "spotify:playlist:x".into(),
            name: "Mine".into(),
            owner: Some("me".into()),
            owned: true,
            saved: Some(true),
            on_profile: true,
            capabilities: Capabilities {
                view: true,
                edit_items: true,
                edit_metadata: true,
                administrate: true,
                publish: true,
                delete: true,
                cancel_membership: false,
            },
            base: Some(Level::Viewer),
            ..PlaylistInfo::default()
        }
    }

    #[test]
    fn builds_the_owner_menu_in_desktop_order() {
        assert_eq!(
            titles(playlist_menu(&owned())),
            [
                "Remove from profile",
                "Edit details",
                "Delete",
                "──",
                "Make private",
                "Invite collaborators"
            ]
        );
        let private = PlaylistInfo {
            base: Some(Level::Blocked),
            ..owned()
        };
        assert_eq!(
            titles(playlist_menu(&private)),
            [
                "Edit details",
                "Delete",
                "──",
                "Make public",
                "Invite collaborators"
            ]
        );
        let unknown = PlaylistInfo {
            base: None,
            on_profile: false,
            ..owned()
        };
        assert!(titles(playlist_menu(&unknown)).contains(&"Make private [x]".to_owned()));
        assert!(titles(playlist_menu(&unknown)).contains(&"Add to profile".to_owned()));
    }

    #[test]
    fn builds_the_follower_menu() {
        let followed = PlaylistInfo {
            uri: "spotify:playlist:y".into(),
            owner: Some("spotify".into()),
            saved: Some(true),
            capabilities: Capabilities {
                view: true,
                publish: true,
                ..Capabilities::default()
            },
            base: Some(Level::Viewer),
            ..PlaylistInfo::default()
        };
        assert_eq!(
            titles(playlist_menu(&followed)),
            ["Remove from Your Library", "Add to profile"]
        );
        let not_saved = PlaylistInfo {
            saved: Some(false),
            ..followed.clone()
        };
        assert_eq!(titles(playlist_menu(&not_saved)), ["Add to Your Library"]);
        let unknown = PlaylistInfo {
            saved: None,
            ..followed
        };
        assert!(titles(playlist_menu(&unknown)).is_empty());
        let member = PlaylistInfo {
            uri: "spotify:playlist:z".into(),
            saved: Some(true),
            members: 3,
            capabilities: Capabilities {
                edit_items: true,
                cancel_membership: true,
                ..Capabilities::default()
            },
            ..PlaylistInfo::default()
        };
        assert_eq!(
            titles(playlist_menu(&member)),
            [
                "Remove from Your Library",
                "──",
                "Leave playlist",
                "Collaborators"
            ]
        );
    }

    #[test]
    fn labels_the_header() {
        assert_eq!(songs_label(1), "1 song");
        assert_eq!(songs_label(12), "12 songs");
    }
}
