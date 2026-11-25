use gpui::{
    Context, Div, FontWeight, MouseButton, Pixels, SharedString, Stateful, Window, div, prelude::*,
    px, rgb, rgba,
};
use gpui_component::{Sizable, input::Input, tooltip::Tooltip};
use spotify_client::{
    entity,
    library::{CUSTOM_ORDER, ItemKind, LibraryItem, PLAYLISTS},
};

use super::{
    state::{LIBRARY_CONTEXT, State},
    view::{LIBRARY_ROW, PLAYLIST_ROW, ROW_BADGES, item_page},
};
use crate::{
    app::OpenMusic,
    areas::playlists,
    icons, menus, overlay,
    ui::{
        Palette,
        components::{ROW_RADIUS, artwork, bone, icon, scroll_sentinel},
        menu::{MenuItem, Place, Row, Target},
        tracks::{RowDrag, RowDrop},
    },
};

const ROW_HEIGHT: f32 = 44.;
const ROW_ART: f32 = 32.;
const INDENT: f32 = 14.;
const LIKED_TOP: u32 = 0x450af5;
const LIKED_BOTTOM: u32 = 0x8e8ee5;

pub(super) fn empty_text(filter: Option<&str>) -> (&'static str, &'static str) {
    match filter {
        Some("Playlists") => ("Create your first playlist", "It's easy, we'll help you"),
        Some("Podcasts & Shows") => (
            "Let's find some podcasts to follow",
            "We'll keep you updated on new episodes",
        ),
        Some(_) => ("Nothing here yet", "Things you save show up here."),
        None => (
            "Your Library is empty",
            "Save songs, albums and playlists to see them here.",
        ),
    }
}

fn custom_order(library: &State) -> bool {
    let filters = &library.query.filters;
    let playlists = filters.is_empty() || (filters.len() == 1 && filters[0] == PLAYLISTS);
    playlists
        && library.query.text.is_none()
        && library
            .page
            .as_ref()
            .and_then(|page| page.sort.as_ref())
            .is_some_and(|sort| sort.id == CUSTOM_ORDER)
}

impl OpenMusic {
    pub fn playing_from(&self, item: &LibraryItem) -> bool {
        self.playback
            .now
            .context_uri
            .as_deref()
            .is_some_and(|context| {
                item.uri == context
                    || (item.kind == ItemKind::LikedSongs && entity::is_liked_songs(context))
            })
    }

    pub fn library_art(&self, item: &LibraryItem, side: Pixels, palette: Palette) -> Div {
        let radius = if item.kind.round() {
            side / 2.
        } else {
            px((f32::from(side) * 0.16).clamp(4., 12.))
        };
        let image = item.image.as_ref().and_then(|url| self.covers.get(url));
        let tile = || {
            div()
                .size(side)
                .flex_shrink_0()
                .rounded(radius)
                .flex()
                .items_center()
                .justify_center()
        };
        match item.kind {
            ItemKind::Folder => tile().bg(rgb(palette.raised)).child(icon(
                icons::FOLDER,
                f32::from(side) * 0.46,
                palette.muted,
            )),
            ItemKind::LikedSongs if image.is_none() => tile()
                .bg(gpui::linear_gradient(
                    135.,
                    gpui::linear_color_stop(rgb(LIKED_TOP), 0.),
                    gpui::linear_color_stop(rgb(LIKED_BOTTOM), 1.),
                ))
                .child(icon(icons::HEART_FILL, f32::from(side) * 0.42, 0xffffff)),
            _ if !item.available => tile().bg(rgba(palette.press)),
            _ => artwork(image, side, radius, palette),
        }
    }

    fn library_tool(
        &self,
        key: &'static str,
        path: &'static str,
        palette: Palette,
        cx: &mut Context<Self>,
        run: impl Fn(&mut OpenMusic, &mut Window, &mut Context<OpenMusic>) + 'static,
    ) -> Stateful<Div> {
        div()
            .id(key)
            .relative()
            .size(px(28.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_center()
            .rounded_full()
            .cursor_pointer()
            .hover(move |button| button.bg(rgba(palette.press)))
            .child(icon(path, 14., palette.muted))
            .on_click(cx.listener(move |this, _, window, cx| {
                cx.stop_propagation();
                run(this, window, cx);
                cx.notify();
            }))
    }

    pub fn create_menu_items(&self, cx: &mut Context<Self>) -> Vec<MenuItem> {
        let jam = cx.is_action_available(&menus::StartJam);
        let mut items = playlists::view::create_menu();
        items.push(
            MenuItem::action("Jam", |_, window, cx| {
                window.dispatch_action(Box::new(menus::StartJam), cx)
            })
            .symbol("person.2.wave.2")
            .subtitle("Listen together from anywhere")
            .enabled(jam),
        );
        items
    }

    pub fn open_create_menu(&mut self, cx: &mut Context<Self>) {
        let items = self.create_menu_items(cx);
        self.show_menu(items, Place::Under("library-create"), None);
    }

    pub fn open_sort_menu(&mut self) {
        let Some(page) = &self.library.page else {
            return;
        };
        let current = page.sort.as_ref().map(|sort| sort.id.clone());
        let mut items = vec![MenuItem::header("Sort by")];
        items.extend(page.sorts.iter().map(|chip| {
            let id = chip.id.clone();
            MenuItem::action(chip.name.clone(), move |app, _, cx| {
                app.set_library_sort(id.clone());
                cx.notify();
            })
            .checked(current.as_deref() == Some(chip.id.as_str()))
        }));
        self.show_menu(items, Place::Under("library-sort"), None);
    }

    fn library_header(&self, palette: Palette, cx: &mut Context<Self>) -> Div {
        let filter = self
            .library
            .filter
            .as_ref()
            .filter(|_| self.library.filter_open);
        div()
            .h(px(30.))
            .flex()
            .items_center()
            .gap_1()
            .map(|row| match filter {
                Some(input) => row.child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .h(px(30.))
                        .flex()
                        .items_center()
                        .gap_1p5()
                        .pl_2p5()
                        .rounded_full()
                        .bg(rgba(palette.press))
                        .child(icon(icons::SEARCH, 12., palette.muted))
                        .child(
                            div().flex_1().min_w_0().child(
                                Input::new(input)
                                    .small()
                                    .appearance(false)
                                    .bordered(false)
                                    .focus_bordered(false),
                            ),
                        )
                        .child(self.library_tool(
                            "library-filter-close",
                            icons::CLOSE,
                            palette,
                            cx,
                            |app, window, cx| app.close_library_filter(window, cx),
                        )),
                ),
                None => row
                    .pl_2p5()
                    .child(
                        div().flex_1().min_w_0().flex().child(
                            div()
                                .id("library-title")
                                .min_w_0()
                                .truncate()
                                .cursor_pointer()
                                .text_size(px(13.5))
                                .font_weight(FontWeight::BOLD)
                                .hover(|title| title.opacity(0.75))
                                .tooltip(|window, cx| {
                                    Tooltip::new("Open Your Library").build(window, cx)
                                })
                                .child("Your Library")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.open_library_page();
                                    cx.notify();
                                })),
                        ),
                    )
                    .child(self.library_tool(
                        "library-filter",
                        icons::SEARCH,
                        palette,
                        cx,
                        |app, window, cx| app.open_library_filter(window, cx),
                    )),
            })
            .child(
                self.library_tool("library-create", icons::PLUS, palette, cx, |app, _, cx| {
                    app.open_create_menu(cx)
                })
                .child(overlay::anchor("library-create")),
            )
    }

    pub(super) fn library_chip(
        &self,
        id: (&'static str, usize),
        label: SharedString,
        chosen: bool,
        palette: Palette,
    ) -> Stateful<Div> {
        div()
            .id(id)
            .h(px(28.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .px_3()
            .rounded_full()
            .cursor_pointer()
            .text_size(px(12.5))
            .font_weight(FontWeight::MEDIUM)
            .map(|chip| {
                if chosen {
                    chip.bg(rgb(palette.accent)).text_color(rgb(0xffffff))
                } else {
                    chip.bg(rgba(palette.press))
                        .hover(move |chip| chip.bg(rgba(palette.line)))
                }
            })
            .child(label)
    }

    fn library_sort(&self, palette: Palette, cx: &mut Context<Self>) -> Stateful<Div> {
        let sort = self
            .library
            .page
            .as_ref()
            .and_then(|page| page.sort.as_ref())
            .map(|sort| sort.name.clone());
        div()
            .id("library-sort")
            .relative()
            .h(px(28.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap_1()
            .pl_2()
            .pr_1p5()
            .rounded_full()
            .cursor_pointer()
            .text_size(px(12.))
            .font_weight(FontWeight::MEDIUM)
            .text_color(rgb(palette.muted))
            .hover(move |button| button.bg(rgba(palette.press)))
            .children(sort.map(|name| div().max_w(px(76.)).truncate().child(name)))
            .child(icon(icons::QUEUE, 13., palette.muted))
            .child(overlay::anchor("library-sort"))
            .on_click(cx.listener(|this, _, _, cx| {
                cx.stop_propagation();
                this.open_sort_menu();
                cx.notify();
            }))
    }

    fn library_chips(&self, palette: Palette, cx: &mut Context<Self>) -> Option<Div> {
        let page = self.library.page.as_ref()?;
        let chips = !page.selected.is_empty() || !page.filters.is_empty();
        if !chips && page.sorts.is_empty() {
            return None;
        }
        let selected = page.selected.iter().enumerate().map(|(index, chip)| {
            self.library_chip(
                ("library-chip-on", index),
                chip.name.clone().into(),
                true,
                palette,
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.keep_library_filters(index);
                cx.notify();
            }))
        });
        let available = page.filters.iter().enumerate().map(|(index, chip)| {
            let id = chip.id.clone();
            self.library_chip(
                ("library-chip", index),
                chip.name.clone().into(),
                false,
                palette,
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.pick_library_filter(id.clone());
                cx.notify();
            }))
        });
        Some(
            div()
                .flex()
                .items_center()
                .gap_1()
                .child(
                    div()
                        .id("library-chips")
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .items_center()
                        .gap_1p5()
                        .overflow_x_scroll()
                        .when(!page.selected.is_empty(), |row| {
                            row.child(
                                div()
                                    .id("library-chip-clear")
                                    .size(px(28.))
                                    .flex_shrink_0()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded_full()
                                    .cursor_pointer()
                                    .bg(rgba(palette.press))
                                    .hover(move |chip| chip.bg(rgba(palette.line)))
                                    .child(icon(icons::CLOSE, 11., palette.text))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.keep_library_filters(0);
                                        cx.notify();
                                    })),
                            )
                        })
                        .children(selected)
                        .children(available),
                )
                .when(!page.sorts.is_empty(), |row| {
                    row.child(self.library_sort(palette, cx))
                }),
        )
    }

    pub(super) fn library_message(
        &self,
        title: String,
        body: &'static str,
        reload: bool,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Div {
        div()
            .flex()
            .flex_col()
            .gap_1()
            .px_2p5()
            .py_3()
            .child(
                div()
                    .text_size(px(13.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title),
            )
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(rgb(palette.muted))
                    .child(body),
            )
            .when(reload, |message| {
                message.child(
                    div()
                        .id("library-reload")
                        .mt_1()
                        .cursor_pointer()
                        .text_size(px(12.5))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(palette.accent))
                        .child("Reload")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.refresh_library();
                            cx.notify();
                        })),
                )
            })
    }

    fn library_skeleton(&self, rows: usize, palette: Palette) -> Div {
        let widths = [120., 90., 140., 104., 128., 84., 112.];
        div()
            .flex()
            .flex_col()
            .gap_0p5()
            .children((0..rows).map(|index| {
                div()
                    .h(px(ROW_HEIGHT))
                    .flex()
                    .items_center()
                    .gap_2p5()
                    .px_1p5()
                    .child(
                        bone("library-row", index * 3, palette)
                            .size(px(ROW_ART))
                            .rounded(px(5.)),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1p5()
                            .child(
                                bone("library-row", index * 3 + 1, palette)
                                    .w(px(widths[index % widths.len()]))
                                    .h(px(11.))
                                    .rounded(px(4.)),
                            )
                            .child(
                                bone("library-row", index * 3 + 2, palette)
                                    .w(px(widths[(index + 3) % widths.len()] * 0.6))
                                    .h(px(9.))
                                    .rounded(px(4.)),
                            ),
                    )
            }))
    }

    fn library_row(
        &self,
        index: usize,
        item: &LibraryItem,
        show_kind: bool,
        reorder: bool,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let row: Row = match self.library.playlist_rows.get(&item.uri) {
            Some(&position) if item.kind == ItemKind::Playlist => (PLAYLIST_ROW, position),
            _ => (LIBRARY_ROW, index),
        };
        let page = item_page(item);
        let selected = page.as_ref() == Some(&self.page);
        let playing = self.playing_from(item);
        let droppable = row.0 == PLAYLIST_ROW || item.kind == ItemKind::LikedSongs;
        let folder = (item.kind == ItemKind::Folder && !self.library.query.flattened())
            .then(|| item.uri.clone());
        let expanded = folder
            .as_ref()
            .is_some_and(|uri| self.library.query.expanded.contains(uri));
        let menu_uri = item.uri.clone();
        let subtitle = item.subtitle(show_kind);
        let draggable = reorder && matches!(item.kind, ItemKind::Playlist | ItemKind::Folder);
        let listed = (item.kind == ItemKind::Playlist)
            .then(|| self.listed_name(&item.uri))
            .flatten();
        let available = item.available || listed.is_some();
        let title = listed.unwrap_or_else(|| item.title());
        div()
            .id(row)
            .h(px(ROW_HEIGHT))
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap_2p5()
            .pl(px(6. + INDENT * item.depth as f32))
            .pr_1()
            .rounded(px(ROW_RADIUS))
            .cursor_pointer()
            .when(selected || self.menu_row(row), |line| {
                line.bg(rgba(palette.press))
            })
            .hover(move |line| line.bg(rgba(palette.press)))
            .when(!available, |line| line.opacity(0.5))
            .on_mouse_down(
                MouseButton::Right,
                self.on_menu(Some(row), move |_| Target::for_uri(&menu_uri), cx),
            )
            .when(droppable, |line| {
                line.drag_over::<RowDrag>(move |line, _, _, _| line.bg(rgba(palette.press)))
                    .on_drop(cx.listener(move |this, drag: &RowDrag, window, cx| {
                        this.drop_rows(drag, RowDrop { row, after: false }, window, cx);
                    }))
            })
            .child(self.library_art(item, px(ROW_ART), palette))
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
                            .gap_1()
                            .min_w_0()
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(13.))
                                    .font_weight(if selected {
                                        FontWeight::SEMIBOLD
                                    } else {
                                        FontWeight::MEDIUM
                                    })
                                    .when(playing, |title| title.text_color(rgb(palette.accent)))
                                    .child(title),
                            )
                            .children(
                                ROW_BADGES
                                    .iter()
                                    .filter_map(|badge| badge(self, item, palette)),
                            ),
                    )
                    .when(item.pinned || !subtitle.is_empty(), |text| {
                        text.child(
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
                                        .text_size(px(11.5))
                                        .text_color(rgb(palette.muted))
                                        .child(subtitle),
                                ),
                        )
                    }),
            )
            .when_some(folder, |line, uri| {
                line.child(
                    div()
                        .id(("library-folder-toggle", index))
                        .size(px(24.))
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .hover(move |button| button.bg(rgba(palette.line)))
                        .child(icon(
                            if expanded {
                                icons::CHEVRON_DOWN
                            } else {
                                icons::CHEVRON_RIGHT
                            },
                            12.,
                            palette.muted,
                        ))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.toggle_library_folder(&uri);
                            cx.notify();
                        })),
                )
            })
            .on_click(cx.listener(move |this, _, _, cx| {
                if let Some(page) = page.clone() {
                    this.navigate(page);
                    cx.notify();
                }
            }))
            .map(|line| {
                if draggable {
                    self.library_drag(line, &item.uri, &item.name, palette, cx)
                } else {
                    line
                }
            })
    }

    fn library_rows(&self, palette: Palette, cx: &mut Context<Self>) -> Stateful<Div> {
        let library = &self.library;
        let list = div()
            .id("library-rows")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&library.scroll)
            .flex()
            .flex_col()
            .gap_0p5();
        if library.items.is_empty() {
            let waiting = library.loading || (library.page.is_none() && library.error.is_none());
            return list.child(if waiting {
                self.library_skeleton(7, palette)
            } else if library.error.is_some() {
                self.library_message(
                    "Something went wrong".into(),
                    "Try reloading Your Library",
                    true,
                    palette,
                    cx,
                )
            } else if let Some(text) = &library.query.text {
                self.library_message(
                    format!("Couldn't find “{text}”"),
                    "Try searching again using a different spelling or keyword.",
                    false,
                    palette,
                    cx,
                )
            } else {
                let (title, body) = empty_text(library.query.filters.first().map(String::as_str));
                self.library_message(title.into(), body, false, palette, cx)
            });
        }
        let show_kind = library.show_kind();
        let reorder = custom_order(library);
        list.children(
            library.items.iter().enumerate().map(|(index, item)| {
                self.library_row(index, item, show_kind, reorder, palette, cx)
            }),
        )
        .child(scroll_sentinel(&library.sentinel))
        .map(|list| {
            if library.error.is_some() {
                list.child(self.library_message(
                    "Something went wrong".into(),
                    "Try reloading Your Library",
                    true,
                    palette,
                    cx,
                ))
            } else if library.loading && library.has_more() {
                list.child(self.library_skeleton(2, palette))
            } else {
                list
            }
        })
    }

    pub fn playlist_list(&self, palette: Palette, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex_1()
            .min_h_0()
            .overflow_hidden()
            .flex()
            .flex_col()
            .gap_2()
            .key_context(LIBRARY_CONTEXT)
            .when_some(self.library.focus.as_ref(), |section, focus| {
                section.track_focus(focus)
            })
            .child(self.library_header(palette, cx))
            .children(self.library_chips(palette, cx))
            .child(self.library_rows(palette, cx))
    }
}
