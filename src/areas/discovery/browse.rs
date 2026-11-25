use std::time::{Duration, Instant};

use spotify_client::{
    browse::{BrowseAll, BrowsePage},
    home::Section,
};

use super::{state::SectionSource, worker::Command};
use crate::app::{OpenMusic, Page, Session};

pub const BROWSE_ALL_STALE: Duration = Duration::from_secs(3600);
const KEPT_PAGES: usize = 6;

#[derive(Default)]
pub struct BrowseAllState {
    pub all: Option<BrowseAll>,
    pub loaded_at: Option<Instant>,
    pub pending: bool,
    pub error: Option<String>,
}

pub struct BrowseState {
    pub uri: String,
    pub page: Option<BrowsePage>,
    pub pending: Option<u32>,
    pub error: Option<String>,
}

pub struct SectionState {
    pub uri: String,
    pub source: SectionSource,
    pub section: Option<Section>,
    pub pending: Option<u32>,
    pub error: Option<String>,
}

pub trait Keyed {
    fn matches(&self, uri: &str, source: Option<SectionSource>) -> bool;
}

impl Keyed for BrowseState {
    fn matches(&self, uri: &str, _source: Option<SectionSource>) -> bool {
        self.uri == uri
    }
}

impl Keyed for SectionState {
    fn matches(&self, uri: &str, source: Option<SectionSource>) -> bool {
        self.uri == uri && source.is_none_or(|source| source == self.source)
    }
}

pub struct Kept<T>(Vec<T>);

impl<T> Default for Kept<T> {
    fn default() -> Self {
        Self(Vec::new())
    }
}

impl<T: Keyed> Kept<T> {
    pub fn get(&self, uri: &str, source: Option<SectionSource>) -> Option<&T> {
        self.0.iter().find(|item| item.matches(uri, source))
    }

    pub fn get_mut(&mut self, uri: &str, source: Option<SectionSource>) -> Option<&mut T> {
        self.0.iter_mut().find(|item| item.matches(uri, source))
    }

    pub fn put(&mut self, item: T, uri: &str, source: Option<SectionSource>) -> &mut T {
        self.0.retain(|kept| !kept.matches(uri, source));
        if self.0.len() >= KEPT_PAGES {
            self.0.remove(0);
        }
        self.0.push(item);
        let last = self.0.len() - 1;
        &mut self.0[last]
    }

    pub fn touch(&mut self, uri: &str, source: Option<SectionSource>) {
        if let Some(index) = self.0.iter().position(|item| item.matches(uri, source)) {
            let item = self.0.remove(index);
            self.0.push(item);
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.0.iter()
    }

    pub fn clear(&mut self) {
        self.0.clear();
    }
}

pub fn append_section(current: &mut Section, more: Section, offset: u32) {
    if offset == 0 {
        *current = more;
    } else {
        current.append(more);
    }
}

pub fn append_page(current: &mut BrowsePage, more: BrowsePage, offset: u32) {
    if offset == 0 {
        *current = more;
    } else {
        current.append(more);
    }
}

impl super::state::State {
    pub fn browse_images(&self) -> impl Iterator<Item = String> + '_ {
        let tiles = self
            .browse_all
            .all
            .iter()
            .flat_map(|all| &all.tiles)
            .filter_map(|card| card.image.clone());
        let pages = self
            .browse_pages
            .iter()
            .filter_map(|state| state.page.as_ref())
            .flat_map(BrowsePage::images);
        let sections = self
            .sections
            .iter()
            .filter_map(|state| state.section.as_ref())
            .flat_map(|section| section.images().collect::<Vec<_>>());
        tiles.chain(pages).chain(sections)
    }

    pub fn known_section(&self, uri: &str, source: SectionSource) -> Option<&Section> {
        match source {
            SectionSource::Home => self.feed.find_section(uri),
            SectionSource::Browse => self
                .browse_pages
                .iter()
                .filter_map(|state| state.page.as_ref())
                .flat_map(|page| &page.sections)
                .find(|section| section.uri == uri),
        }
    }
}

impl OpenMusic {
    pub fn load_browse_all(&mut self, force: bool) {
        let state = &mut self.discovery.browse_all;
        let fresh = state
            .loaded_at
            .is_some_and(|at| at.elapsed() < BROWSE_ALL_STALE);
        if state.pending || (fresh && !force) || !matches!(self.session, Session::Connected) {
            return;
        }
        state.pending = true;
        self.send(Command::BrowseAll);
    }

    pub fn on_browse_all(&mut self, result: Result<BrowseAll, String>) {
        let state = &mut self.discovery.browse_all;
        state.pending = false;
        match result {
            Ok(all) if !all.tiles.is_empty() => {
                state.all = Some(all);
                state.loaded_at = Some(Instant::now());
                state.error = None;
            }
            Ok(_) => state.error = Some("Spotify sent no categories.".to_owned()),
            Err(error) => state.error = Some(error),
        }
    }

    pub fn restore_browse_all(&mut self, all: BrowseAll) {
        let state = &mut self.discovery.browse_all;
        if state.all.is_none() && !all.tiles.is_empty() {
            state.all = Some(all);
        }
    }

    pub fn load_browse(&mut self, uri: &str) {
        let pages = &mut self.discovery.browse_pages;
        let usable = pages
            .get(uri, None)
            .is_some_and(|state| state.page.is_some() || state.pending.is_some());
        if usable {
            pages.touch(uri, None);
            return;
        }
        pages.put(
            BrowseState {
                uri: uri.to_owned(),
                page: None,
                pending: Some(0),
                error: None,
            },
            uri,
            None,
        );
        self.send(Command::Browse {
            uri: uri.to_owned(),
            offset: 0,
        });
    }

    pub fn load_section(&mut self, uri: &str, source: SectionSource) {
        let sections = &mut self.discovery.sections;
        let usable = sections
            .get(uri, Some(source))
            .is_some_and(|state| state.section.is_some() || state.pending.is_some());
        if usable {
            sections.touch(uri, Some(source));
            return;
        }
        sections.put(
            SectionState {
                uri: uri.to_owned(),
                source,
                section: None,
                pending: Some(0),
                error: None,
            },
            uri,
            Some(source),
        );
        self.send(match source {
            SectionSource::Home => Command::HomeSection(uri.to_owned()),
            SectionSource::Browse => Command::BrowseSection {
                uri: uri.to_owned(),
                offset: 0,
            },
        });
    }

    pub fn more_page_results(&mut self) -> bool {
        match self.page.clone() {
            Page::Browse(uri) => {
                let Some(state) = self.discovery.browse_pages.get_mut(&uri, None) else {
                    return false;
                };
                let Some(next) = state.page.as_ref().and_then(|page| page.next) else {
                    return false;
                };
                if state.pending.is_some() || state.error.is_some() {
                    return false;
                }
                state.pending = Some(next);
                state.error = None;
                self.send(Command::Browse { uri, offset: next });
                true
            }
            Page::Section {
                uri,
                source: SectionSource::Browse,
            } => {
                let source = Some(SectionSource::Browse);
                let Some(state) = self.discovery.sections.get_mut(&uri, source) else {
                    return false;
                };
                let Some(next) = state.section.as_ref().and_then(|section| section.next) else {
                    return false;
                };
                if state.pending.is_some() || state.error.is_some() {
                    return false;
                }
                state.pending = Some(next);
                state.error = None;
                self.send(Command::BrowseSection { uri, offset: next });
                true
            }
            _ => false,
        }
    }

    pub fn retry_page_results(&mut self) -> bool {
        match self.page.clone() {
            Page::Browse(uri) => {
                if let Some(state) = self.discovery.browse_pages.get_mut(&uri, None) {
                    state.error = None;
                }
            }
            Page::Section { uri, source } => {
                if let Some(state) = self.discovery.sections.get_mut(&uri, Some(source)) {
                    state.error = None;
                }
            }
            _ => return false,
        }
        self.more_page_results()
    }

    pub fn on_browse(&mut self, uri: String, offset: u32, result: Result<Box<BrowsePage>, String>) {
        let Some(state) = self.discovery.browse_pages.get_mut(&uri, None) else {
            return;
        };
        if state.pending != Some(offset) {
            return;
        }
        state.pending = None;
        match (result, &mut state.page) {
            (Ok(page), Some(current)) => append_page(current, *page, offset),
            (Ok(page), None) => state.page = Some(*page),
            (Err(error), _) => state.error = Some(error),
        }
    }

    pub fn on_section(
        &mut self,
        uri: String,
        source: SectionSource,
        offset: u32,
        result: Result<Box<Section>, String>,
    ) {
        let Some(state) = self.discovery.sections.get_mut(&uri, Some(source)) else {
            return;
        };
        if state.pending != Some(offset) {
            return;
        }
        state.pending = None;
        match (result, &mut state.section) {
            (Ok(section), Some(current)) => append_section(current, *section, offset),
            (Ok(section), None) => state.section = Some(*section),
            (Err(error), _) => state.error = Some(error),
        }
    }

    pub fn browse_fragment(&self) -> String {
        let mut parts = vec![format!(
            "browse_all={}",
            self.discovery
                .browse_all
                .all
                .as_ref()
                .map_or(0, |all| all.tiles.len())
        )];
        match &self.page {
            Page::Browse(uri) => {
                let state = self.discovery.browse_pages.get(uri, None);
                let page = state.and_then(|state| state.page.as_ref());
                parts.push(format!(
                    "browse={uri} browse_title={:?} browse_sections={} browse_total={} browse_next={:?} browse_pending={:?}",
                    page.map_or("", |page| page.title.as_str()),
                    page.map_or(0, |page| page.sections.len()),
                    page.map_or(0, |page| page.total),
                    page.and_then(|page| page.next),
                    state.and_then(|state| state.pending),
                ));
            }
            Page::Section { uri, source } => {
                let state = self.discovery.sections.get(uri, Some(*source));
                let section = state.and_then(|state| state.section.as_ref());
                parts.push(format!(
                    "section={uri} section_title={:?} section_cards={} section_total={} section_next={:?} section_pending={:?}",
                    section.map_or("", |section| section.title.as_str()),
                    section.map_or(0, |section| section.cards.len()),
                    section.map_or(0, |section| section.total),
                    section.and_then(|section| section.next),
                    state.and_then(|state| state.pending),
                ));
            }
            _ => {}
        }
        parts.join(" ")
    }
}

#[cfg(test)]
mod tests {
    use spotify_client::{
        entity::{Card, Kind},
        home::Layout,
    };

    use super::*;

    fn card(id: &str) -> Card {
        crate::ui::cards::card(
            format!("spotify:playlist:{id}"),
            Kind::Playlist,
            id,
            "",
            None,
        )
    }

    fn section(ids: &[&str], next: Option<u32>) -> Section {
        Section {
            uri: "spotify:section:s".into(),
            title: "Shelf".into(),
            subtitle: String::new(),
            layout: Layout::Shelf,
            header: None,
            cards: ids.iter().map(|id| card(id)).collect(),
            total: 60,
            next,
        }
    }

    fn state(uri: &str, source: SectionSource) -> SectionState {
        SectionState {
            uri: uri.into(),
            source,
            section: None,
            pending: None,
            error: None,
        }
    }

    #[test]
    fn keeps_a_few_pages_most_recent_last() {
        let mut kept = Kept::default();
        for index in 0..(KEPT_PAGES + 2) {
            let uri = format!("spotify:section:{index}");
            kept.put(
                state(&uri, SectionSource::Browse),
                &uri,
                Some(SectionSource::Browse),
            );
        }
        assert_eq!(kept.iter().count(), KEPT_PAGES);
        assert!(kept.get("spotify:section:0", None).is_none());
        assert!(kept.get("spotify:section:2", None).is_some());
        kept.touch("spotify:section:2", None);
        assert_eq!(kept.iter().last().unwrap().uri, "spotify:section:2");
        assert!(
            kept.get("spotify:section:2", Some(SectionSource::Home))
                .is_none()
        );
        let uri = "spotify:section:2";
        kept.put(
            state(uri, SectionSource::Home),
            uri,
            Some(SectionSource::Home),
        );
        assert!(kept.get(uri, Some(SectionSource::Browse)).is_some());
        assert!(kept.get(uri, Some(SectionSource::Home)).is_some());
    }

    #[test]
    fn appends_later_pages_and_replaces_the_first() {
        let mut current = section(&["a", "b"], Some(20));
        append_section(&mut current, section(&["b", "c"], Some(40)), 20);
        let ids: Vec<&str> = current
            .cards
            .iter()
            .map(|card| card.title.as_str())
            .collect();
        assert_eq!(ids, ["a", "b", "c"]);
        assert_eq!(current.next, Some(40));
        append_section(&mut current, section(&["c"], Some(60)), 40);
        assert_eq!(current.next, None);
        append_section(&mut current, section(&["z"], None), 0);
        assert_eq!(current.cards.len(), 1);
    }
}
