use std::{
    collections::{HashMap, HashSet},
    sync::{Mutex, OnceLock, PoisonError},
};

use gpui::{
    AnyElement, Context, Div, FontWeight, MouseButton, Pixels, SharedString, Stateful, Window, div,
    hsla, prelude::*, px, rgb, rgba,
};
use gpui_component::tooltip::Tooltip;
use spotify_client::{
    Track,
    blend::{self, Attribution, Invitation, InvitationState, Member},
    catalog::Collection,
    entity,
    library::{ItemKind, LibraryItem},
    playlist::{self, ItemPosition, Level, RootEntry, RootNode, RootPlace, Rootlist},
};

use super::state::{BlendWork, PlaylistInfo, owner_label, source_name};
use crate::{
    app::{OpenMusic, Page},
    areas::playback::view::shuffle_spec,
    icons,
    overlay::Label,
    ui::{
        Palette,
        cards::Face,
        components::{artwork, bone, icon, link, scroll_sentinel, skeleton_header, skeleton_rows},
        dock::DockItem,
        menu::{Group, MenuItem, Row, Surface, Target, TrackTarget},
        sidebar::NavEntry,
        tracks::RowOptions,
    },
};

pub const TRACK_ROW: &str = "collection-track";
pub const ADD_MENU: &str = "add-to-playlist";
pub const ATTRIBUTION_WIDTH: f32 = 96.;
pub const ATTRIBUTION_SHOWN: usize = 5;
const UNDATED_FORMATS: [&str; 6] = [
    "daily-mix",
    "my-mix",
    "car-mix",
    "personalized-podcasts",
    "daylist",
    "inspiredby-mix",
];
const CREATE_TITLE: &str = "Invite friends";
const CREATE_SUBTITLE: &str = "Pick a friend to create a Blend with\u{2014}a playlist that shows how your music taste matches up.";
const CREATE_FOOTNOTE: &str = "Note: You may invite up to 10 people. Connected people will see your profile picture and username. Inviting friends will create playlists and use other recommendation features that match your taste.";
const CREATE_BUTTON: &str = "Invite";

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AddSource {
    pub uris: Vec<String>,
    pub name: Option<String>,
    pub exclude: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LibraryDrag {
    pub uri: String,
    pub name: String,
}

struct LibraryGhost {
    name: SharedString,
    folder: bool,
    palette: Palette,
}

impl Render for LibraryGhost {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .h(px(32.))
            .max_w(px(240.))
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .rounded_full()
            .bg(rgb(self.palette.raised))
            .border_1()
            .border_color(rgba(self.palette.line))
            .shadow_lg()
            .text_size(px(13.))
            .font_weight(FontWeight::MEDIUM)
            .text_color(rgb(self.palette.text))
            .child(icon(
                if self.folder {
                    icons::FOLDER
                } else {
                    icons::NOTE
                },
                14.,
                self.palette.accent,
            ))
            .child(div().min_w_0().truncate().child(self.name.clone()))
    }
}

pub fn shows_added(format: Option<&str>) -> bool {
    !blend::is_blend(format) && !format.is_some_and(|format| UNDATED_FORMATS.contains(&format))
}

pub fn static_label(text: &str) -> &'static str {
    static LABELS: OnceLock<Mutex<HashSet<&'static str>>> = OnceLock::new();
    let mut labels = LABELS
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if let Some(found) = labels.get(text) {
        return found;
    }
    let leaked: &'static str = Box::leak(text.to_owned().into_boxed_str());
    labels.insert(leaked);
    leaked
}

fn faces(members: &[Member]) -> Vec<Face> {
    members
        .iter()
        .map(|member| Face {
            name: member.label().to_owned(),
            image: member.image.clone(),
        })
        .collect()
}

fn blend_glyph(side: f32, color: u32) -> Div {
    let ring = |left: f32| {
        div()
            .absolute()
            .top_0()
            .left(px(left))
            .size(px(side))
            .rounded_full()
            .border_1()
            .border_color(rgb(color))
    };
    div()
        .relative()
        .flex_shrink_0()
        .w(px(side * 1.55))
        .h(px(side))
        .child(ring(0.))
        .child(ring(side * 0.55))
}

pub fn create_menu() -> Vec<MenuItem> {
    vec![
        MenuItem::action("Playlist", |app, _, _| {
            app.create_playlist(None, None, Vec::new(), true)
        })
        .symbol("music.note.list")
        .subtitle("Create a playlist with songs or episodes"),
        MenuItem::action("Blend", |app, _, _| app.create_blend())
            .symbol("person.2")
            .subtitle("Combine your friends' tastes into a playlist"),
        MenuItem::action("Folder", |app, _, _| {
            app.create_folder(None, Vec::new(), true)
        })
        .symbol("folder")
        .subtitle("Organize your playlists"),
    ]
}

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

        let blend_members = if is_playlist && blend::is_blend_list(&collection.metadata) {
            blend::members(&collection.metadata)
        } else {
            Vec::new()
        };
        let meta = if !blend_members.is_empty() {
            let creators_uri = loaded.clone();
            div()
                .w_full()
                .flex()
                .items_center()
                .gap_1()
                .min_w_0()
                .child(
                    div()
                        .id("collection-creators")
                        .flex()
                        .items_center()
                        .gap_1p5()
                        .min_w_0()
                        .cursor_pointer()
                        .child(self.facepile(
                            &faces(&blend_members[..blend_members.len().min(3)]),
                            px(24.),
                            3,
                            palette,
                        ))
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .text_size(px(13.5))
                                .font_weight(FontWeight::SEMIBOLD)
                                .hover(|text| text.underline())
                                .child(blend::creators(&blend_members)),
                        )
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.show_blend_members(&creators_uri, window, cx);
                            cx.notify();
                        })),
                )
                .child(
                    div()
                        .flex_shrink_0()
                        .text_size(px(13.5))
                        .text_color(rgb(palette.muted))
                        .child("•"),
                )
                .child(
                    div()
                        .flex_shrink_0()
                        .text_size(px(13.5))
                        .text_color(rgb(palette.muted))
                        .child(summary(collection)),
                )
        } else if is_playlist {
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
            .filter(|_| blend_members.is_empty())
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
                let shuffle = shuffle_spec(
                    self.context_shuffle(loaded),
                    self.context_shuffle_allowed(loaded),
                    self.context_smart_available(loaded),
                );
                actions.child(
                    self.glass_control(
                        "shuffle-collection",
                        shuffle,
                        !self.modal_open(),
                        palette,
                        cx,
                    )
                    .size(px(36.)),
                )
            })
            .children(self.library_button(loaded, palette, cx))
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
            let editable_rows = info
                .as_ref()
                .is_some_and(|info| info.capabilities.edit_items);
            let options = RowOptions {
                numbered: album,
                context: Some(loaded.clone()),
                show_album: !album,
                show_added: is_playlist && shows_added(collection.format.as_deref()),
                editable: editable_rows,
                draggable: true,
                ..RowOptions::default()
            };
            self.track_list(&collection.tracks, TRACK_ROW, 0, &options, palette, cx)
                .when(collection.next.is_some(), |list| {
                    list.child(scroll_sentinel(&self.playlists.sentinel))
                        .when(self.playlists.loading_more, |list| {
                            list.child(skeleton_rows("collection-more", 3, album, palette))
                        })
                })
        };
        page.child(header).child(rows)
    }
}

impl OpenMusic {
    pub fn library_drag(
        &self,
        row: Stateful<Div>,
        uri: &str,
        name: &str,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let folder = playlist::folder_hash(uri).is_some();
        if !folder && playlist::canonical(uri).is_none() {
            return row;
        }
        let drag = LibraryDrag {
            uri: uri.to_owned(),
            name: name.to_owned(),
        };
        row.relative()
            .on_drag(drag, move |drag, _, _, cx| {
                let name: SharedString = drag.name.clone().into();
                cx.new(|_| LibraryGhost {
                    name,
                    folder,
                    palette,
                })
            })
            .child(self.library_zone(uri, false, folder, palette, cx))
            .child(self.library_zone(uri, true, folder, palette, cx))
    }

    fn library_zone(
        &self,
        uri: &str,
        after: bool,
        folder: bool,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let target = uri.to_owned();
        let into = folder && after;
        div()
            .absolute()
            .left_0()
            .right_0()
            .h_1_2()
            .map(|zone| if after { zone.bottom_0() } else { zone.top_0() })
            .drag_over::<LibraryDrag>(move |style, _, _, _| {
                if into {
                    style.bg(rgba(palette.press))
                } else if after {
                    style.border_b_2().border_color(rgb(palette.accent))
                } else {
                    style.border_t_2().border_color(rgb(palette.accent))
                }
            })
            .on_drop(cx.listener(move |this, drag: &LibraryDrag, _, cx| {
                this.drop_library(drag, &target, after);
                cx.notify();
            }))
    }

    pub fn playlist_glyph(&self, format: Option<&str>, palette: Palette) -> Option<AnyElement> {
        blend::is_blend(format).then(|| blend_glyph(11., palette.muted).into_any_element())
    }

    fn playlist_format(&self, uri: &str) -> Option<String> {
        if let Some(root) = self
            .playlists
            .rootlist
            .as_ref()
            .and_then(|list| list.playlist(uri))
        {
            return root.format.clone();
        }
        let index = *self.library.playlist_rows.get(uri)?;
        self.library.playlists.get(index)?.format.clone()
    }

    fn create_invitation(&self) -> Invitation {
        let account = self.account();
        let username = account
            .map(|account| account.username.clone())
            .or_else(|| self.playlists.user.clone())
            .unwrap_or_default();
        let me = Member {
            name: self.inviter_name().unwrap_or_else(|| username.clone()),
            image: account.and_then(|account| account.avatar.clone()),
            username,
        };
        Invitation {
            state: InvitationState::Pending,
            title: CREATE_TITLE.to_owned(),
            subtitle: CREATE_SUBTITLE.to_owned(),
            footnote: Some(CREATE_FOOTNOTE.to_owned()),
            button: Some(CREATE_BUTTON.to_owned()),
            members: vec![me],
            members_title: None,
            recipient: None,
            playlist_uri: None,
        }
    }

    fn big_face(&self, member: Option<&Member>, side: Pixels, palette: Palette) -> Div {
        let face = match member {
            Some(member) => self.avatar(member.label(), member.image.as_deref(), side, palette),
            None => div()
                .size(side)
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(rgb(palette.raised))
                .child(icon(icons::PLUS, 44., palette.muted)),
        };
        div()
            .rounded_full()
            .border_4()
            .border_color(rgb(palette.canvas))
            .shadow_lg()
            .child(face)
    }

    fn invitation_art(
        &self,
        invitation: &Invitation,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let side = px(128.);
        match invitation.state {
            InvitationState::Full | InvitationState::Deleted => div()
                .size(side)
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(rgb(palette.raised))
                .child(icon(
                    if invitation.state == InvitationState::Full {
                        icons::PERSON
                    } else {
                        icons::NOTE
                    },
                    52.,
                    palette.muted,
                ))
                .into_any_element(),
            InvitationState::ReadyExisting => {
                let rows = invitation
                    .members
                    .iter()
                    .enumerate()
                    .map(|(index, member)| {
                        let page = member.uri().map(Page::Profile);
                        div()
                            .id(("blend-invitation-member", index))
                            .flex()
                            .items_center()
                            .gap_3()
                            .px_2()
                            .py_1()
                            .rounded(px(8.))
                            .child(self.avatar(
                                member.label(),
                                member.image.as_deref(),
                                px(32.),
                                palette,
                            ))
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(13.5))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(member.label().to_owned()),
                            )
                            .when_some(page, |row, page| {
                                row.cursor_pointer()
                                    .hover(move |row| row.bg(rgba(palette.press)))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.navigate(page.clone());
                                        cx.notify();
                                    }))
                            })
                    });
                div()
                    .w(px(320.))
                    .flex()
                    .flex_col()
                    .gap_1()
                    .p_3()
                    .rounded(px(14.))
                    .bg(rgba(palette.fill))
                    .border_1()
                    .border_color(rgba(palette.rim))
                    .when_some(invitation.members_title.clone(), |list, heading| {
                        list.child(
                            div()
                                .px_2()
                                .pb_1()
                                .text_size(px(11.5))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(palette.muted))
                                .child(heading),
                        )
                    })
                    .children(rows)
                    .into_any_element()
            }
            _ => {
                let first = invitation.members.first().or(invitation.recipient.as_ref());
                let second = invitation
                    .members
                    .first()
                    .and(invitation.recipient.as_ref());
                div()
                    .flex()
                    .items_center()
                    .child(self.big_face(first, side, palette))
                    .child(self.big_face(second, side, palette).ml(side * -0.22))
                    .into_any_element()
            }
        }
    }

    pub fn blend_invitation_page(
        &self,
        id: &str,
        _card_side: Pixels,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let page = div()
            .flex()
            .flex_col()
            .gap_7()
            .child(div().child(self.back_button(palette, cx)));
        let created;
        let invitation = if id.is_empty() {
            created = self.create_invitation();
            &created
        } else {
            match &self.playlists.invitation {
                Some((loaded, Ok(invitation))) if loaded == id => invitation,
                Some((loaded, Err(error))) if loaded == id => {
                    return page
                        .child(self.page_status(error.clone(), palette))
                        .into_any_element();
                }
                _ => {
                    return page
                        .child(
                            div()
                                .w_full()
                                .flex()
                                .flex_col()
                                .items_center()
                                .gap_4()
                                .pt_4()
                                .child(
                                    bone("blend-invitation", 0, palette)
                                        .size(px(128.))
                                        .rounded_full(),
                                )
                                .child(
                                    bone("blend-invitation", 1, palette)
                                        .w(px(260.))
                                        .h(px(22.))
                                        .rounded(px(6.)),
                                )
                                .child(
                                    bone("blend-invitation", 2, palette)
                                        .w(px(340.))
                                        .h(px(13.))
                                        .rounded(px(5.)),
                                ),
                        )
                        .into_any_element();
                }
            }
        };
        if let InvitationState::Other(_) = invitation.state {
            return page
                .child(self.page_status("Something went wrong.", palette))
                .into_any_element();
        }
        let joining = matches!(&self.playlists.blend_work, Some(BlendWork::Joining(joining)) if joining == id);
        let inviting = self.playlists.blend_work == Some(BlendWork::Inviting);
        let actionable = invitation.state.joins() || invitation.state.invites();
        let busy = if joining {
            Some("Joining…")
        } else if inviting && invitation.state.invites() {
            Some("Creating invite link…")
        } else {
            None
        };
        let column = div()
            .w_full()
            .flex()
            .flex_col()
            .items_center()
            .gap_4()
            .pt_2()
            .child(self.invitation_art(invitation, palette, cx))
            .child(
                div()
                    .max_w(px(560.))
                    .text_center()
                    .text_size(px(30.))
                    .font_weight(FontWeight::BOLD)
                    .child(invitation.title.clone()),
            )
            .when(!invitation.subtitle.is_empty(), |column| {
                column.child(
                    div()
                        .max_w(px(480.))
                        .text_center()
                        .text_size(px(14.))
                        .text_color(rgb(palette.muted))
                        .child(invitation.subtitle.clone()),
                )
            })
            .when_some(
                invitation
                    .button
                    .as_deref()
                    .filter(|_| actionable)
                    .map(static_label),
                |column, label| {
                    column.child(div().pt_2().child(self.text_control(
                        "blend-action",
                        label,
                        true,
                        palette,
                        cx,
                    )))
                },
            )
            .when_some(busy, |column, busy| {
                column.child(
                    div()
                        .text_size(px(12.5))
                        .text_color(rgb(palette.muted))
                        .child(busy),
                )
            })
            .when_some(invitation.footnote.clone(), |column, footnote| {
                column.child(
                    div()
                        .max_w(px(440.))
                        .pt_2()
                        .text_center()
                        .text_size(px(11.5))
                        .text_color(rgb(palette.muted))
                        .child(footnote),
                )
            });
        page.child(column).into_any_element()
    }

    fn attribution_cell(
        &self,
        found: Option<&Vec<Attribution>>,
        list: &HashMap<String, String>,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Div {
        let side = px(22.);
        let cell = div()
            .w(px(ATTRIBUTION_WIDTH))
            .flex_shrink_0()
            .flex()
            .items_center();
        let Some(found) = found else {
            return cell;
        };
        cell.children(found.iter().take(ATTRIBUTION_SHOWN).enumerate().map(
            |(index, attribution)| {
                let name: SharedString = attribution.label().to_owned().into();
                let reason: SharedString = attribution.reason(list).into();
                let page = attribution.uri().map(Page::Profile);
                div()
                    .id(("attribution", index))
                    .flex_shrink_0()
                    .rounded_full()
                    .border_2()
                    .border_color(rgb(palette.canvas))
                    .when(index > 0, |face| face.ml(side * -0.3))
                    .child(self.avatar(&name, attribution.image.as_deref(), side, palette))
                    .tooltip(move |window, cx| {
                        let (name, reason) = (name.clone(), reason.clone());
                        Tooltip::element(move |_, _| {
                            div()
                                .flex()
                                .flex_col()
                                .child(div().font_weight(FontWeight::SEMIBOLD).child(name.clone()))
                                .child(div().opacity(0.7).child(reason.clone()))
                        })
                        .build(window, cx)
                    })
                    .when_some(page, |face, page| {
                        face.cursor_pointer()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.navigate(page.clone());
                                cx.notify();
                            }))
                    })
            },
        ))
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
                .symbol("checkmark.circle.fill")
            } else {
                MenuItem::action("Add to Your Library", move |app, _, _| {
                    app.follow_playlist(&uri, true)
                })
                .symbol("plus.circle")
            },
        ));
    }
    let mut edit = Vec::new();
    if saved && info.capabilities.publish && info.base != Some(Level::Blocked) {
        let uri = uri.clone();
        let public = info.on_profile;
        edit.push(
            MenuItem::action(
                if public {
                    "Remove from profile"
                } else {
                    "Add to profile"
                },
                move |app, _, _| app.set_playlist_on_profile(&uri, !public),
            )
            .symbol(if public {
                "person.crop.circle.badge.minus"
            } else {
                "person.crop.circle.badge.plus"
            }),
        );
    }
    if info.capabilities.edit_metadata {
        let uri = uri.clone();
        edit.push(
            MenuItem::action("Edit details", move |app, window, cx| {
                app.edit_details(&uri, window, cx)
            })
            .symbol("pencil"),
        );
    }
    if saved && deletable {
        let uri = uri.clone();
        edit.push(
            MenuItem::action("Delete", move |app, window, cx| {
                app.delete_playlist(&uri, window, cx)
            })
            .symbol("trash")
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
                })
                .symbol("globe"),
                Some(_) => MenuItem::action("Make private", move |app, window, cx| {
                    app.set_playlist_private(&uri, true, window, cx)
                })
                .symbol("lock"),
                None => MenuItem::action("Make private", |_, _, _| {})
                    .symbol("lock")
                    .enabled(false),
            }
        };
        sharing.push(privacy);
        let uri = uri.clone();
        sharing.push(
            MenuItem::action("Invite collaborators", move |app, _, _| {
                app.invite_collaborators(uri.clone())
            })
            .symbol("person.badge.plus"),
        );
    }
    if info.capabilities.cancel_membership {
        let uri = uri.clone();
        sharing.push(
            MenuItem::action("Leave playlist", move |app, window, cx| {
                app.leave_playlist(uri.clone(), window, cx)
            })
            .symbol("rectangle.portrait.and.arrow.right"),
        );
    }
    if info.members > 1 {
        sharing.push(
            MenuItem::action("Collaborators", move |app, window, cx| {
                app.show_collaborators(&uri, window, cx)
            })
            .symbol("person.2"),
        );
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

fn add_targets(
    nodes: Vec<RootNode>,
    list: &Rootlist,
    source: &AddSource,
    checks: &HashMap<String, bool>,
) -> Vec<MenuItem> {
    nodes
        .into_iter()
        .filter_map(|node| match node {
            RootNode::Playlist { uri, index } => {
                let Some(RootEntry::Playlist(found)) = list.entries.get(index) else {
                    return None;
                };
                if !found.capabilities.edit_items || source.exclude.as_deref() == Some(uri.as_str())
                {
                    return None;
                }
                let title = if found.name.is_empty() {
                    "Untitled".to_owned()
                } else {
                    found.name.clone()
                };
                let checked = checks.get(&uri).copied().unwrap_or(false);
                let sources = source.uris.clone();
                Some(
                    MenuItem::action(title, move |app, _, _| {
                        app.add_to_playlist(&uri, sources.clone(), ItemPosition::Bottom)
                    })
                    .checked(checked),
                )
            }
            RootNode::Folder(folder) => {
                let children = add_targets(folder.children, list, source, checks);
                (!children.is_empty())
                    .then(|| MenuItem::submenu(folder.name, children).symbol("folder"))
            }
        })
        .collect()
}

pub fn add_menu(app: &OpenMusic, source: &AddSource) -> Vec<MenuItem> {
    let checks = app.playlists.curates(&source.uris);
    let (sources, name) = (source.uris.clone(), source.name.clone());
    let mut items = vec![
        MenuItem::action("New playlist", move |app, _, _| {
            app.create_playlist_from(sources.clone(), name.clone(), None)
        })
        .symbol("plus"),
    ];
    let targets = match (&app.playlists.rootlist, &app.playlists.user) {
        (Some(list), Some(user)) => add_targets(list.tree(user), list, source, &checks),
        _ => vec![MenuItem::note("Loading playlists…")],
    };
    if !targets.is_empty() {
        items.push(MenuItem::Separator);
        items.extend(targets);
    }
    items
}

fn add_submenu(title: &str, app: &OpenMusic, source: &AddSource) -> MenuItem {
    MenuItem::submenu(title, add_menu(app, source))
        .symbol("plus")
        .search("Find a playlist")
}

fn track_items(target: &TrackTarget, app: &OpenMusic) -> Vec<(Group, MenuItem)> {
    let uri = target.track.uri.clone();
    if !playlist::addable(&uri) {
        return Vec::new();
    }
    let title = Some(target.track.title.clone()).filter(|title| !title.is_empty());
    let source = AddSource {
        uris: vec![uri.clone()],
        name: title.clone(),
        exclude: None,
    };
    if target.surface != Surface::NowPlaying {
        app.playlists.wanted.replace(Some(source.clone()));
    }
    let mut items = vec![(
        Group::Edit,
        add_submenu("Add to playlist", app, &source).id(ADD_MENU),
    )];
    if let (Some(uid), Some(context)) = (&target.uid, &target.context)
        && !matches!(target.provider.as_deref(), Some("autoplay" | "queue"))
        && app
            .playlist_info(context)
            .is_some_and(|info| info.capabilities.edit_items)
    {
        let (context, uid) = (context.clone(), uid.clone());
        items.push((
            Group::Edit,
            MenuItem::action("Remove from this playlist", move |app, _, _| {
                app.remove_from_playlist(&context, vec![uid.clone()])
            })
            .symbol("trash")
            .key("backspace"),
        ));
    }
    if !uri.starts_with("spotify:local:") {
        items.push((
            Group::Edit,
            MenuItem::action("Choose playlists…", move |app, window, cx| {
                app.open_save_to(vec![uri.clone()], title.clone(), window, cx)
            })
            .symbol("checkmark.circle"),
        ));
    }
    items
}

fn folder_moves(app: &OpenMusic, uri: &str) -> Vec<MenuItem> {
    let parent = app.playlists.parent_folder(uri);
    let parent_hash = parent.as_deref().and_then(playlist::folder_hash);
    let moving = app.playlists.folder(uri);
    let mut folders = Vec::new();
    let mut names: Vec<String> = Vec::new();
    for (depth, folder) in app.playlists.folders() {
        names.truncate(depth);
        let parent_name = names.last().cloned();
        names.push(folder.name.clone());
        let inside = moving.as_ref().is_some_and(|moving| {
            folder.start >= moving.start && folder.start < moving.start + moving.length
        });
        if inside {
            continue;
        }
        let current = parent_hash == Some(folder.hash.as_str());
        let (moved, into) = (uri.to_owned(), folder.uri.clone());
        let mut item = MenuItem::action(folder.name.clone(), move |app, _, _| {
            app.move_to(&moved, RootPlace::Into(into.clone()))
        })
        .symbol("folder")
        .checked(current)
        .enabled(!current);
        if let Some(parent_name) = parent_name {
            item = item.subtitle(format!("In {parent_name}"));
        }
        folders.push(item);
    }
    let mut entries = Vec::new();
    if moving.is_none() {
        let playlists = vec![uri.to_owned()];
        entries.push(
            MenuItem::action("Create folder", move |app, _, _| {
                app.create_folder(None, playlists.clone(), true)
            })
            .symbol("folder.badge.plus"),
        );
        if !folders.is_empty() {
            entries.push(MenuItem::Separator);
        }
    }
    entries.extend(folders);
    let mut items = Vec::new();
    if !entries.is_empty() {
        let title = if parent.is_some() {
            "Move to folder"
        } else {
            "Add to folder"
        };
        items.push(
            MenuItem::submenu(title, entries)
                .symbol("folder")
                .search("Find a folder"),
        );
    }
    if parent.is_some() {
        let moved = uri.to_owned();
        items.push(
            MenuItem::action("Remove from folders", move |app, _, _| {
                app.move_to(&moved, RootPlace::Start)
            })
            .symbol("folder.badge.minus"),
        );
    }
    items
}

fn folder_items(app: &OpenMusic, uri: &str) -> Vec<(Group, MenuItem)> {
    if app.playlists.folder(uri).is_none() {
        return Vec::new();
    }
    let (rename, delete, create, folder) = (
        uri.to_owned(),
        uri.to_owned(),
        uri.to_owned(),
        uri.to_owned(),
    );
    let mut items = vec![
        MenuItem::action("Rename", move |app, window, cx| {
            app.rename_folder(&rename, None, window, cx)
        })
        .symbol("pencil"),
        MenuItem::action("Delete", move |app, window, cx| {
            app.delete_folder(&delete, window, cx)
        })
        .symbol("trash"),
        MenuItem::Separator,
        MenuItem::action("Create playlist", move |app, _, _| {
            app.create_playlist(None, Some(create.clone()), Vec::new(), true)
        })
        .symbol("music.note.list"),
        MenuItem::action("Create folder", move |app, _, _| {
            app.create_folder(Some(folder.clone()), Vec::new(), true)
        })
        .symbol("folder.badge.plus"),
    ];
    items.extend(folder_moves(app, uri));
    items.into_iter().map(|item| (Group::Edit, item)).collect()
}

fn playlist_items(app: &OpenMusic, uri: &str) -> Vec<(Group, MenuItem)> {
    let Some(info) = app.playlist_info(uri) else {
        return Vec::new();
    };
    if app.playlists.wants_permissions(&info) {
        app.playlists
            .wanted_permissions
            .replace(Some(info.uri.clone()));
    }
    let mut items = playlist_menu(&info);
    if info.saved == Some(true) {
        items.extend(
            folder_moves(app, &info.uri)
                .into_iter()
                .map(|item| (Group::Edit, item)),
        );
    }
    let source = AddSource {
        uris: vec![info.uri.clone()],
        name: Some(info.name.clone()).filter(|name| !name.is_empty()),
        exclude: Some(info.uri.clone()),
    };
    items.push((
        Group::Edit,
        add_submenu("Add to other playlist", app, &source),
    ));
    items
}

pub fn menu_items(target: &Target, app: &OpenMusic) -> Vec<(Group, MenuItem)> {
    match target {
        Target::Track(track) => track_items(track, app),
        Target::Album { uri } | Target::Episode { uri } => {
            let source = AddSource {
                uris: vec![uri.clone()],
                name: source_name(app, std::slice::from_ref(uri)),
                exclude: None,
            };
            vec![(Group::Edit, add_submenu("Add to playlist", app, &source))]
        }
        Target::Playlist { uri } => playlist_items(app, uri),
        Target::Folder { uri } => folder_items(app, uri),
        Target::Artist { .. } | Target::Show { .. } | Target::User { .. } => Vec::new(),
    }
}

pub fn blend_badge(app: &OpenMusic, item: &LibraryItem, palette: Palette) -> Option<AnyElement> {
    if item.kind != ItemKind::Playlist {
        return None;
    }
    app.playlist_glyph(app.playlist_format(&item.uri).as_deref(), palette)
}

pub fn nav_entries(_app: &OpenMusic) -> Vec<NavEntry> {
    Vec::new()
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
    app: &OpenMusic,
    track: &Track,
    row: Row,
    options: &RowOptions,
    palette: Palette,
    cx: &mut Context<OpenMusic>,
) -> Vec<AnyElement> {
    if row.0 != TRACK_ROW {
        return Vec::new();
    }
    let Some((loaded, Ok(collection))) = &app.playlists.collection else {
        return Vec::new();
    };
    let attributed =
        !collection.attributions.is_empty() || blend::is_blend_list(&collection.metadata);
    if !attributed || options.context.as_deref() != Some(loaded.as_str()) {
        return Vec::new();
    }
    let key = track.uid.as_ref().unwrap_or(&track.uri);
    vec![
        app.attribution_cell(
            collection.attributions.get(key),
            &collection.metadata,
            palette,
            cx,
        )
        .into_any_element(),
    ]
}

pub fn row_dimmed(_app: &OpenMusic, _track: &Track, _row: Row, _options: &RowOptions) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use spotify_client::{catalog::Capabilities, playlist::RootPlaylist};

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

    fn root(uri: &str, name: &str, editable: bool) -> RootEntry {
        RootEntry::Playlist(RootPlaylist {
            uri: uri.into(),
            key: uri.into(),
            name: name.into(),
            capabilities: Capabilities {
                edit_items: editable,
                ..Capabilities::default()
            },
            ..RootPlaylist::default()
        })
    }

    fn outline(items: &[MenuItem], depth: usize, lines: &mut Vec<String>) {
        for item in items {
            let indent = "  ".repeat(depth);
            match item {
                MenuItem::Action { title, checked, .. } => lines.push(format!(
                    "{indent}{title}{}",
                    if *checked { " ✓" } else { "" }
                )),
                MenuItem::Submenu { title, items, .. } => {
                    lines.push(format!("{indent}{title} ▸"));
                    outline(items, depth + 1, lines);
                }
                MenuItem::Separator => lines.push(format!("{indent}──")),
                MenuItem::Note(text) | MenuItem::Header(text) => {
                    lines.push(format!("{indent}{text}"))
                }
                MenuItem::Volume(value) => lines.push(format!("{indent}[volume {value:.2}]")),
            }
        }
    }

    #[test]
    fn lists_editable_playlists_inside_their_folders() {
        let list = Rootlist {
            revision: String::new(),
            entries: vec![
                root("spotify:playlist:a", "Mine", true),
                RootEntry::Start {
                    hash: "f1".into(),
                    name: "Trips".into(),
                },
                root("spotify:playlist:b", "Editorial", false),
                root("spotify:playlist:c", "Road", true),
                RootEntry::End { hash: "f1".into() },
                RootEntry::Start {
                    hash: "f2".into(),
                    name: "Empty".into(),
                },
                root("spotify:playlist:d", "Followed", false),
                RootEntry::End { hash: "f2".into() },
                root("spotify:playlist:e", "", true),
            ],
        };
        let source = AddSource {
            uris: vec!["spotify:track:t".into()],
            name: Some("Song".into()),
            exclude: Some("spotify:playlist:e".into()),
        };
        let checks = HashMap::from([("spotify:playlist:c".to_owned(), true)]);
        let mut lines = Vec::new();
        outline(
            &add_targets(list.tree("someone"), &list, &source, &checks),
            0,
            &mut lines,
        );
        assert_eq!(lines, ["Mine", "Trips ▸", "  Road ✓"]);
        let source = AddSource {
            exclude: None,
            ..source
        };
        let mut lines = Vec::new();
        outline(
            &add_targets(list.tree("someone"), &list, &source, &HashMap::new()),
            0,
            &mut lines,
        );
        assert_eq!(lines, ["Mine", "Trips ▸", "  Road", "Untitled"]);
    }

    #[test]
    fn labels_the_header() {
        assert_eq!(songs_label(1), "1 song");
        assert_eq!(songs_label(12), "12 songs");
    }

    #[test]
    fn hides_date_added_where_the_desktop_does() {
        assert!(shows_added(None));
        assert!(shows_added(Some("editorial")));
        assert!(!shows_added(Some("blend")));
        assert!(!shows_added(Some("celebrity-blend")));
        assert!(!shows_added(Some("daily-mix")));
        assert!(!shows_added(Some("daylist")));
    }

    #[test]
    fn offers_playlist_blend_and_folder_to_create() {
        let menu: Vec<(String, Option<String>)> = create_menu()
            .into_iter()
            .map(|item| match item {
                MenuItem::Action {
                    title, subtitle, ..
                } => (title, subtitle),
                _ => ("?".to_owned(), None),
            })
            .collect();
        assert_eq!(
            menu,
            [
                (
                    "Playlist".to_owned(),
                    Some("Create a playlist with songs or episodes".to_owned())
                ),
                (
                    "Blend".to_owned(),
                    Some("Combine your friends' tastes into a playlist".to_owned())
                ),
                (
                    "Folder".to_owned(),
                    Some("Organize your playlists".to_owned())
                ),
            ]
        );
    }

    #[test]
    fn keeps_one_copy_of_each_button_label() {
        let first = static_label("Join");
        let again = static_label(&String::from("Join"));
        assert_eq!(first, "Join");
        assert!(std::ptr::eq(first, again));
        assert_ne!(static_label("Invite"), first);
    }
}
