use gpui::{
    AnyElement, Context, Div, FontWeight, Pixels, Stateful, div, prelude::*, px, rgb, rgba,
};
use spotify_client::library::{LibraryItem, LibraryPage, LibraryQuery};

use super::{
    sidebar::empty_text,
    state::{apply_pins, hide_removing, merge},
    worker::Command,
};
use crate::{
    app::{OpenMusic, Page},
    icons, overlay,
    ui::{
        Palette,
        components::{Sentinel, icon, scroll_sentinel},
        menu::{MenuItem, Place},
    },
};

const SORT_ANCHOR: &str = "library-page-sort";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shelf {
    All,
    Playlists,
    Podcasts,
    Artists,
    Albums,
    Audiobooks,
    Authors,
    Events,
}

impl Shelf {
    pub const SHELVES: [Self; 8] = [
        Self::All,
        Self::Playlists,
        Self::Podcasts,
        Self::Artists,
        Self::Albums,
        Self::Audiobooks,
        Self::Authors,
        Self::Events,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Playlists => "playlists",
            Self::Podcasts => "podcasts",
            Self::Artists => "artists",
            Self::Albums => "albums",
            Self::Audiobooks => "audiobooks",
            Self::Authors => "authors",
            Self::Events => "events",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::SHELVES.into_iter().find(|shelf| shelf.name() == name)
    }

    pub fn filter(self) -> Option<&'static str> {
        Some(match self {
            Self::All => return None,
            Self::Playlists => "Playlists",
            Self::Podcasts => "Podcasts & Shows",
            Self::Artists => "Artists",
            Self::Albums => "Albums",
            Self::Audiobooks => "Audiobooks",
            Self::Authors => "Authors",
            Self::Events => "Events",
        })
    }

    pub fn from_filter(filter: Option<&str>) -> Self {
        Self::SHELVES
            .into_iter()
            .find(|shelf| shelf.filter() == filter)
            .unwrap_or(Self::All)
    }

    fn query(self) -> LibraryQuery {
        LibraryQuery {
            filters: self.filter().map(str::to_owned).into_iter().collect(),
            ..LibraryQuery::default()
        }
    }
}

#[derive(Default)]
pub struct Grid {
    pub query: LibraryQuery,
    pub page: Option<LibraryPage>,
    pub items: Vec<LibraryItem>,
    pub loading: bool,
    pub error: Option<String>,
    pub request: u64,
    pub next_offset: u32,
    pub sentinel: Sentinel,
}

impl Grid {
    pub fn has_more(&self) -> bool {
        self.page
            .as_ref()
            .is_some_and(|page| !page.fallback && self.next_offset < page.total)
    }

    fn shelf(&self) -> Shelf {
        Shelf::from_filter(self.query.filters.first().map(String::as_str))
    }
}

impl OpenMusic {
    pub fn load_grid(&mut self) {
        let grid = &mut self.library.grid;
        grid.request += 1;
        grid.loading = true;
        grid.error = None;
        let command = Command::Grid {
            query: grid.query.clone(),
            offset: 0,
            request: grid.request,
        };
        self.send(command);
    }

    pub fn more_grid(&mut self) {
        let grid = &mut self.library.grid;
        if grid.loading || grid.error.is_some() || !grid.has_more() {
            return;
        }
        grid.request += 1;
        grid.loading = true;
        let command = Command::Grid {
            query: grid.query.clone(),
            offset: grid.next_offset,
            request: grid.request,
        };
        self.send(command);
    }

    pub(super) fn show_shelf(&mut self, shelf: Shelf) {
        let grid = &mut self.library.grid;
        let changed = grid.query.filters.first().map(String::as_str) != shelf.filter();
        if changed {
            grid.query = shelf.query();
            grid.page = None;
            grid.items.clear();
            grid.error = None;
        }
        let idle = grid.page.is_none() || grid.error.is_some();
        if changed || (idle && !grid.loading) {
            self.load_grid();
        }
    }

    pub fn open_shelf(&mut self, shelf: Shelf) {
        if self.page == Page::Library(shelf) {
            self.show_shelf(shelf);
        } else {
            self.navigate(Page::Library(shelf));
        }
    }

    pub fn open_library_page(&mut self) {
        let query = LibraryQuery {
            filters: self.library.query.filters.clone(),
            sort: self.library.query.sort.clone(),
            ..LibraryQuery::default()
        };
        let shelf = Shelf::from_filter(query.filters.first().map(String::as_str));
        if self.library.grid.query != query {
            let grid = &mut self.library.grid;
            grid.query = query;
            grid.page = None;
            grid.items.clear();
            grid.error = None;
            self.load_grid();
        }
        self.open_shelf(shelf);
    }

    pub fn set_grid_query(&mut self, query: LibraryQuery) {
        if query == self.library.grid.query {
            return;
        }
        self.library.grid.query = query;
        self.load_grid();
        if matches!(self.page, Page::Library(_)) {
            self.page = Page::Library(self.library.grid.shelf());
        }
    }

    fn pick_grid_filter(&mut self, id: String) {
        let mut query = self.library.grid.query.clone();
        query.filters.push(id);
        self.set_grid_query(query);
    }

    fn keep_grid_filters(&mut self, count: usize) {
        let grid = &self.library.grid;
        let selected: Vec<String> = match &grid.page {
            Some(page) if !page.selected.is_empty() => {
                page.selected.iter().map(|chip| chip.id.clone()).collect()
            }
            _ => grid.query.filters.clone(),
        };
        let mut query = grid.query.clone();
        query.filters = selected.into_iter().take(count).collect();
        self.set_grid_query(query);
    }

    pub fn set_grid_sort(&mut self, sort: Option<String>) {
        let mut query = self.library.grid.query.clone();
        query.sort = sort;
        self.set_grid_query(query);
    }

    pub(super) fn on_grid(
        &mut self,
        request: u64,
        query: LibraryQuery,
        offset: u32,
        result: Result<LibraryPage, String>,
    ) {
        let grid = &mut self.library.grid;
        if request != grid.request {
            return;
        }
        grid.loading = false;
        match result {
            Ok(mut page) => {
                grid.next_offset = merge(&mut grid.items, &mut page, offset);
                grid.query = query;
                grid.page = Some(page);
                grid.error = None;
                apply_pins(&mut self.library);
                hide_removing(self);
                if matches!(self.page, Page::Library(_)) {
                    self.page = Page::Library(self.library.grid.shelf());
                }
                if self.library.grid.items.is_empty() {
                    self.more_grid();
                }
            }
            Err(message) => grid.error = Some(message),
        }
    }

    pub fn open_grid_sort_menu(&mut self) {
        let Some(page) = &self.library.grid.page else {
            return;
        };
        let current = page.sort.as_ref().map(|sort| sort.id.clone());
        let mut items = vec![MenuItem::header("Sort by")];
        items.extend(page.sorts.iter().map(|chip| {
            let id = chip.id.clone();
            MenuItem::action(chip.name.clone(), move |app, _, cx| {
                app.set_grid_sort(Some(id.clone()));
                cx.notify();
            })
            .checked(current.as_deref() == Some(chip.id.as_str()))
        }));
        self.show_menu(items, Place::Under(SORT_ANCHOR), None);
    }

    fn grid_sort(&self, palette: Palette, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        let page = self.library.grid.page.as_ref()?;
        if page.sorts.is_empty() {
            return None;
        }
        let name = page.sort.as_ref().map(|sort| sort.name.clone());
        Some(
            div()
                .id(SORT_ANCHOR)
                .relative()
                .h(px(30.))
                .flex_shrink_0()
                .flex()
                .items_center()
                .gap_1p5()
                .px_2p5()
                .rounded_full()
                .cursor_pointer()
                .text_size(px(12.5))
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(palette.muted))
                .hover(move |button| button.bg(rgba(palette.press)))
                .children(name)
                .child(icon(icons::QUEUE, 13., palette.muted))
                .child(overlay::anchor(SORT_ANCHOR))
                .on_click(cx.listener(|this, _, _, cx| {
                    cx.stop_propagation();
                    this.open_grid_sort_menu();
                    cx.notify();
                })),
        )
    }

    fn grid_chips(&self, palette: Palette, cx: &mut Context<Self>) -> Option<Div> {
        let page = self.library.grid.page.as_ref()?;
        if page.selected.is_empty() && page.filters.is_empty() {
            return None;
        }
        let selected = page.selected.iter().enumerate().map(|(index, chip)| {
            self.library_chip(
                ("library-page-chip-on", index),
                chip.name.clone().into(),
                true,
                palette,
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.keep_grid_filters(index);
                cx.notify();
            }))
        });
        let available = page.filters.iter().enumerate().map(|(index, chip)| {
            let id = chip.id.clone();
            self.library_chip(
                ("library-page-chip", index),
                chip.name.clone().into(),
                false,
                palette,
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.pick_grid_filter(id.clone());
                cx.notify();
            }))
        });
        Some(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_2()
                .when(!page.selected.is_empty(), |row| {
                    row.child(
                        div()
                            .id("library-page-chip-clear")
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
                                this.keep_grid_filters(0);
                                cx.notify();
                            })),
                    )
                })
                .children(selected)
                .children(available),
        )
    }

    pub fn library_page(
        &self,
        _shelf: Shelf,
        card_side: Pixels,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let grid = &self.library.grid;
        let header = div()
            .flex()
            .items_center()
            .gap_3()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(34.))
                    .font_weight(FontWeight::BOLD)
                    .child("Your Library"),
            )
            .children(self.grid_sort(palette, cx));
        let body = if grid.items.is_empty() {
            let waiting = grid.loading || (grid.page.is_none() && grid.error.is_none());
            if waiting {
                self.grid_skeleton(8, card_side, palette)
            } else if grid.error.is_some() {
                self.library_message(
                    "Something went wrong".into(),
                    "Try reloading Your Library",
                    true,
                    palette,
                    cx,
                )
            } else {
                let (title, body) = empty_text(grid.query.filters.first().map(String::as_str));
                self.library_message(title.into(), body, false, palette, cx)
            }
        } else {
            div()
                .flex()
                .flex_col()
                .gap_4()
                .child(self.library_grid(&grid.items, "library-card", card_side, palette, cx))
                .child(scroll_sentinel(&grid.sentinel))
                .map(|body| {
                    if grid.error.is_some() {
                        body.child(self.library_message(
                            "Something went wrong".into(),
                            "Try reloading Your Library",
                            true,
                            palette,
                            cx,
                        ))
                    } else if grid.has_more() {
                        body.child(self.grid_skeleton(4, card_side, palette))
                    } else {
                        body
                    }
                })
        };
        div()
            .flex()
            .flex_col()
            .gap_6()
            .child(div().child(self.back_button(palette, cx)))
            .child(header)
            .children(self.grid_chips(palette, cx))
            .child(body)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shelves_map_to_library_filters() {
        for shelf in Shelf::SHELVES {
            assert_eq!(Shelf::from_name(shelf.name()), Some(shelf));
            assert_eq!(Shelf::from_filter(shelf.filter()), shelf);
        }
        assert_eq!(Shelf::Podcasts.filter(), Some("Podcasts & Shows"));
        assert_eq!(Shelf::from_filter(Some("Something new")), Shelf::All);
        assert_eq!(Shelf::from_name("nope"), None);
        assert_eq!(Shelf::Albums.query().filters, vec!["Albums".to_owned()]);
        assert!(Shelf::All.query().filters.is_empty());
        let mut grid = Grid {
            query: Shelf::Artists.query(),
            ..Grid::default()
        };
        assert_eq!(grid.shelf(), Shelf::Artists);
        assert!(!grid.has_more());
        grid.page = Some(LibraryPage {
            total: 80,
            ..LibraryPage::default()
        });
        grid.next_offset = 50;
        assert!(grid.has_more());
    }
}
