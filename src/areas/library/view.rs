use gpui::{AnyElement, Context, Div, FontWeight, MouseButton, Pixels, div, prelude::*, px, rgb};
use spotify_client::{
    Track,
    library::{ItemKind, LibraryItem},
};

use super::{liked::LIKED_ROW, saves};
use crate::{
    app::{OpenMusic, Page, page_for},
    icons,
    ui::{
        Palette,
        components::{bone, icon, scroll_sentinel},
        dock::DockItem,
        menu::{Group, MenuItem, Row, Target},
        sidebar::NavEntry,
        tracks::RowOptions,
    },
};

pub const PLAYLIST_ROW: &str = "playlist";
pub const LIBRARY_ROW: &str = "library";

pub type RowBadge = fn(&OpenMusic, &LibraryItem, Palette) -> Option<AnyElement>;

pub const ROW_BADGES: &[RowBadge] = &[];

pub fn item_page(item: &LibraryItem) -> Option<Page> {
    if !item.available {
        return None;
    }
    let uri = item.uri.clone();
    match item.kind {
        ItemKind::LikedSongs => Some(Page::Liked),
        ItemKind::YourEpisodes => Some(Page::YourEpisodes),
        ItemKind::Folder => Some(Page::Folder(uri)),
        ItemKind::Show | ItemKind::Audiobook => Some(Page::Show(uri)),
        ItemKind::Artist => Some(Page::Artist(uri)),
        ItemKind::Playlist | ItemKind::Album => Some(Page::Collection(uri)),
        ItemKind::PreRelease | ItemKind::Event => page_for(&uri),
    }
}

fn pin_noun(kind: ItemKind) -> &'static str {
    match kind {
        ItemKind::Playlist | ItemKind::LikedSongs | ItemKind::YourEpisodes => "playlist",
        ItemKind::Album => "album",
        ItemKind::Artist => "artist",
        ItemKind::Show => "podcast",
        ItemKind::Audiobook => "audiobook",
        ItemKind::Folder => "folder",
        ItemKind::PreRelease => "upcoming album",
        ItemKind::Event => "event",
    }
}

impl OpenMusic {
    pub fn library_grid(
        &self,
        items: &[LibraryItem],
        prefix: &'static str,
        card_side: Pixels,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Div {
        div()
            .flex()
            .flex_wrap()
            .gap_4()
            .children(items.iter().enumerate().map(|(index, item)| {
                let page = item_page(item);
                let uri = item.uri.clone();
                let row = (prefix, index);
                div()
                    .id(row)
                    .w(card_side)
                    .flex()
                    .flex_col()
                    .gap_2()
                    .cursor_pointer()
                    .when(!item.available, |card| card.opacity(0.5))
                    .child(self.library_art(item, card_side, palette).shadow_md())
                    .child(
                        div()
                            .w_full()
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .truncate()
                                    .when(self.playing_from(item), |title| {
                                        title.text_color(rgb(palette.accent))
                                    })
                                    .child(item.title()),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .min_w_0()
                                    .when(item.pinned, |line| {
                                        line.child(icon(icons::PIN, 10., palette.accent))
                                    })
                                    .child(
                                        div()
                                            .min_w_0()
                                            .truncate()
                                            .text_size(px(12.))
                                            .text_color(rgb(palette.muted))
                                            .child(item.subtitle(true)),
                                    ),
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
                        self.on_menu(Some(row), move |_| Target::for_uri(&uri), cx),
                    )
            }))
    }

    pub(super) fn grid_skeleton(&self, count: usize, card_side: Pixels, palette: Palette) -> Div {
        div()
            .flex()
            .flex_wrap()
            .gap_4()
            .children((0..count).map(|index| {
                div()
                    .w(card_side)
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        bone("folder-card", index * 3, palette)
                            .size(card_side)
                            .rounded(px(12.)),
                    )
                    .child(
                        bone("folder-card", index * 3 + 1, palette)
                            .w(card_side * 0.7)
                            .h(px(12.))
                            .rounded(px(4.)),
                    )
                    .child(
                        bone("folder-card", index * 3 + 2, palette)
                            .w(card_side * 0.45)
                            .h(px(10.))
                            .rounded(px(4.)),
                    )
            }))
    }
}

impl OpenMusic {
    pub fn folder_page(
        &self,
        uri: &str,
        card_side: Pixels,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let folder = self
            .library
            .folder
            .as_ref()
            .filter(|folder| folder.uri == uri);
        let name = self.library.folder_name(uri);
        let detail = self
            .library
            .item(uri)
            .map(|item| item.detail.clone())
            .filter(|detail| !detail.is_empty())
            .or_else(|| {
                let folder = folder?;
                let page = folder.page.as_ref()?;
                let playlists = folder
                    .items
                    .iter()
                    .filter(|item| item.kind != ItemKind::Folder)
                    .count() as u64;
                let folders = folder.items.len() as u64 - playlists;
                Some(if page.total as usize > folder.items.len() {
                    spotify_client::library::count_label(u64::from(page.total), "item", "items")
                } else {
                    spotify_client::library::folder_detail(playlists, folders)
                })
            })
            .unwrap_or_default();
        let target_uri = uri.to_owned();
        let has_menu = self.has_menu(&Target::Folder {
            uri: uri.to_owned(),
        });
        let header = div()
            .flex()
            .items_center()
            .gap_6()
            .on_mouse_down(
                MouseButton::Right,
                self.on_menu(
                    None,
                    move |_| {
                        Some(Target::Folder {
                            uri: target_uri.clone(),
                        })
                    },
                    cx,
                ),
            )
            .child(
                div()
                    .size(px(184.))
                    .flex_shrink_0()
                    .rounded(px(16.))
                    .bg(rgb(palette.raised))
                    .shadow_lg()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(icon(icons::FOLDER, 72., palette.muted)),
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
                            .child("Folder"),
                    )
                    .child(
                        div()
                            .text_size(px(34.))
                            .font_weight(FontWeight::BOLD)
                            .truncate()
                            .w_full()
                            .child(name.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(13.5))
                            .text_color(rgb(palette.muted))
                            .child(detail),
                    )
                    .when(has_menu, |text| {
                        text.child(div().mt_3().child(self.menu_button(
                            "more-folder",
                            &name,
                            palette,
                            cx,
                        )))
                    }),
            );
        let body = match folder {
            None => self.grid_skeleton(4, card_side, palette),
            Some(folder) if folder.items.is_empty() && folder.loading => {
                self.grid_skeleton(4, card_side, palette)
            }
            Some(folder) if folder.items.is_empty() => match &folder.error {
                Some(error) => self.page_status(error.clone(), palette),
                None => div()
                    .py_8()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_size(px(15.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("This folder looks empty"),
                    )
                    .child(
                        div()
                            .text_size(px(13.))
                            .text_color(rgb(palette.muted))
                            .child("Start adding playlists by dragging and dropping"),
                    ),
            },
            Some(folder) => div()
                .flex()
                .flex_col()
                .gap_4()
                .child(self.library_grid(&folder.items, "folder-item", card_side, palette, cx))
                .child(scroll_sentinel(&folder.sentinel))
                .when(folder.has_more(), |body| {
                    body.child(self.grid_skeleton(4, card_side, palette))
                }),
        };
        div()
            .flex()
            .flex_col()
            .gap_7()
            .child(div().child(self.back_button(palette, cx)))
            .child(header)
            .child(body)
            .into_any_element()
    }
}

pub fn menu_items(target: &Target, app: &OpenMusic) -> Vec<(Group, MenuItem)> {
    if matches!(target, Target::Track(_) | Target::User { .. }) {
        return Vec::new();
    }
    let Some(item) = app.library_item(target.uri()) else {
        return Vec::new();
    };
    if !item.pinned && !item.pinnable {
        return Vec::new();
    }
    let uri = item.uri.clone();
    let pin = !item.pinned;
    let noun = pin_noun(item.kind);
    let title = if pin {
        format!("Pin {noun}")
    } else {
        format!("Unpin {noun}")
    };
    let pending = app.library.pins.contains_key(&uri);
    vec![(
        Group::Library,
        MenuItem::action(title, move |app, _, cx| {
            app.set_pinned(&uri, pin);
            cx.notify();
        })
        .symbol(if pin { "pin" } else { "pin.slash" })
        .enabled(!pending),
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
    saves::dock_heart(app, palette, cx).into_iter().collect()
}

pub fn row_accessories(
    app: &OpenMusic,
    track: &Track,
    _row: Row,
    _options: &RowOptions,
    palette: Palette,
    cx: &mut Context<OpenMusic>,
) -> Vec<AnyElement> {
    saves::row_heart(app, track, palette, cx)
        .into_iter()
        .collect()
}

pub fn row_dimmed(app: &OpenMusic, track: &Track, row: Row, _options: &RowOptions) -> bool {
    row.0 == LIKED_ROW && app.library.liked.unplayable.contains(&track.uri)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn library_rows_open_the_right_page() {
        let item = |uri: &str, kind| LibraryItem::new(uri, kind, "Name");
        assert_eq!(
            item_page(&item("spotify:collection:tracks", ItemKind::LikedSongs)),
            Some(Page::Liked)
        );
        assert_eq!(
            item_page(&item(
                "spotify:playlist:37i9dQZF1FgnTBfUlzkeKt",
                ItemKind::YourEpisodes
            )),
            Some(Page::YourEpisodes)
        );
        assert_eq!(
            item_page(&item("spotify:user:someone:folder:1a2b", ItemKind::Folder)),
            Some(Page::Folder("spotify:user:someone:folder:1a2b".into()))
        );
        assert_eq!(
            item_page(&item("spotify:show:x", ItemKind::Audiobook)),
            Some(Page::Show("spotify:show:x".into()))
        );
        assert_eq!(
            item_page(&item("spotify:album:x", ItemKind::Album)),
            Some(Page::Collection("spotify:album:x".into()))
        );
        let mut gone = item("spotify:playlist:x", ItemKind::Playlist);
        gone.available = false;
        assert_eq!(item_page(&gone), None);
    }
}
