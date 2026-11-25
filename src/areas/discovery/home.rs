use std::{
    collections::HashMap,
    path::Path,
    time::{Duration, Instant},
};

use gpui::{Context, SharedString, Window};
use spotify_client::{
    Track,
    connect::{self, PlayOptions},
    entity::{self, Card, Kind},
    home::{Chip, Home, Section},
};

use super::{state::SectionSource, worker::Command};
use crate::app::{OpenMusic, Page, Session};

pub const HOME_STALE: Duration = Duration::from_secs(300);
pub const SHELF_CARDS: usize = 4;
const KEPT_FACETS: usize = 6;
pub const LIKED_SONGS: &str = "spotify:collection:tracks";
pub const HOME_ORIGIN: &str = "home";
pub const SEARCH_ORIGIN: &str = "search";
pub const ARTIST_ORIGIN: &str = "artist";

pub fn origin_for(page: &Page) -> &'static str {
    match page {
        Page::Home
        | Page::Section {
            source: SectionSource::Home,
            ..
        } => HOME_ORIGIN,
        Page::Search(_)
        | Page::Browse(_)
        | Page::Section {
            source: SectionSource::Browse,
            ..
        } => SEARCH_ORIGIN,
        Page::Artist(_) => ARTIST_ORIGIN,
        _ => connect::ORIGIN,
    }
}

#[derive(Default)]
pub struct HomeFeed {
    pub facet: String,
    pub shown: String,
    pub feeds: HashMap<String, (Option<Instant>, Home)>,
    pub pending: Option<String>,
    pub error: Option<String>,
    pub refresh_failed: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ChipEntry {
    pub id: String,
    pub label: SharedString,
    pub selected: bool,
}

pub fn chip_entries(chips: &[Chip], facet: &str) -> Vec<ChipEntry> {
    let mut entries = vec![ChipEntry {
        id: String::new(),
        label: "All".into(),
        selected: facet.is_empty(),
    }];
    for chip in chips {
        let child = chip.sub.iter().any(|sub| sub.id == facet);
        let selected = chip.id == facet || child;
        entries.push(ChipEntry {
            id: chip.id.clone(),
            label: chip.label.clone().into(),
            selected,
        });
        if selected {
            entries.extend(chip.sub.iter().map(|sub| ChipEntry {
                id: sub.id.clone(),
                label: sub.label.clone().into(),
                selected: sub.id == facet,
            }));
        }
    }
    entries
}

pub fn next_facet(chips: &[Chip], facet: &str, picked: &str) -> String {
    if picked != facet {
        return picked.to_owned();
    }
    chips
        .iter()
        .find(|chip| chip.sub.iter().any(|sub| sub.id == picked))
        .map(|parent| parent.id.clone())
        .unwrap_or_default()
}

pub fn card_plays(card: &Card) -> bool {
    card.playable
        && matches!(
            card.kind,
            Kind::Track
                | Kind::Album
                | Kind::Playlist
                | Kind::Artist
                | Kind::LikedSongs
                | Kind::Show
                | Kind::Episode
        )
}

pub fn card_track(card: &Card) -> Track {
    Track {
        uri: card.uri.clone(),
        title: card.title.clone(),
        artist: card.subtitle.clone(),
        cover: card.image.clone(),
        explicit: card.explicit,
        ..Track::default()
    }
}

pub fn same_context(card_uri: &str, context: &str) -> bool {
    card_uri == context || (entity::is_liked_songs(card_uri) && entity::is_liked_songs(context))
}

impl HomeFeed {
    pub fn current(&self) -> Option<(&Home, bool)> {
        if let Some((_, home)) = self.feeds.get(&self.facet) {
            return Some((home, false));
        }
        if self.pending.is_some() {
            return self.feeds.get(&self.shown).map(|(_, home)| (home, true));
        }
        None
    }

    pub fn chips(&self) -> &[Chip] {
        let chips = self
            .current()
            .map(|(home, _)| home.chips.as_slice())
            .unwrap_or_default();
        if !chips.is_empty() {
            return chips;
        }
        self.feeds
            .get("")
            .map(|(_, home)| home.chips.as_slice())
            .unwrap_or_default()
    }

    pub fn needs_load(&self, force: bool) -> bool {
        if self.pending.as_deref() == Some(self.facet.as_str()) {
            return false;
        }
        match self.feeds.get(&self.facet) {
            Some((Some(at), _)) => force || at.elapsed() >= HOME_STALE,
            _ => true,
        }
    }

    pub fn select(&mut self, facet: String) {
        if facet == self.facet {
            return;
        }
        if self.feeds.contains_key(&self.facet) {
            self.shown = self.facet.clone();
        }
        self.facet = facet;
        self.error = None;
        self.refresh_failed = false;
    }

    pub fn insert(&mut self, facet: String, loaded: Option<Instant>, home: Home) {
        if !self.feeds.contains_key(&facet) && self.feeds.len() >= KEPT_FACETS {
            let oldest = self
                .feeds
                .iter()
                .filter(|(key, _)| !key.is_empty() && **key != self.facet)
                .min_by_key(|(_, (at, _))| *at)
                .map(|(key, _)| key.clone());
            if let Some(oldest) = oldest {
                self.feeds.remove(&oldest);
            }
        }
        self.feeds.insert(facet, (loaded, home));
    }

    pub fn find_section(&self, uri: &str) -> Option<&Section> {
        self.feeds.values().find_map(|(_, home)| home.section(uri))
    }

    pub fn images(&self) -> impl Iterator<Item = String> + '_ {
        self.feeds.values().flat_map(|(_, home)| home.images())
    }
}

impl OpenMusic {
    pub fn load_home(&mut self, force: bool) {
        if !matches!(self.session, Session::Connected) || !self.discovery.feed.needs_load(force) {
            return;
        }
        let facet = self.discovery.feed.facet.clone();
        self.discovery.feed.pending = Some(facet.clone());
        self.send(Command::Home(facet));
    }

    pub fn select_home_facet(&mut self, facet: String) {
        self.discovery.feed.select(facet);
        self.load_home(false);
    }

    pub fn pick_home_chip(&mut self, picked: &str) {
        let feed = &self.discovery.feed;
        let facet = next_facet(feed.chips(), &feed.facet, picked);
        self.select_home_facet(facet);
    }

    pub fn on_home(&mut self, facet: String, result: Result<Box<Home>, String>) {
        let feed = &mut self.discovery.feed;
        if feed.pending.as_ref() == Some(&facet) {
            feed.pending = None;
        }
        match result {
            Ok(home) => {
                if facet == feed.facet {
                    feed.error = None;
                    feed.refresh_failed = false;
                }
                feed.insert(facet, Some(Instant::now()), *home);
            }
            Err(error) if facet == feed.facet => {
                if feed.feeds.contains_key(&facet) {
                    feed.refresh_failed = true;
                } else if !facet.is_empty() {
                    feed.select(String::new());
                    self.load_home(false);
                } else {
                    feed.error = Some(error);
                }
            }
            Err(_) => {}
        }
    }

    pub fn restore_home(&mut self, home: Home) {
        let feed = &mut self.discovery.feed;
        if !feed.feeds.contains_key("") {
            feed.insert(String::new(), None, home);
        }
    }

    pub fn context_is_current(&self, uri: &str) -> bool {
        self.playback
            .now
            .context_uri
            .as_deref()
            .is_some_and(|context| same_context(uri, context))
    }

    pub fn card_is_current(&self, card: &Card) -> bool {
        self.context_is_current(&card.uri)
            || (matches!(card.kind, Kind::Track | Kind::Episode)
                && self
                    .playback
                    .now
                    .track
                    .as_ref()
                    .is_some_and(|track| track.uri == card.uri))
    }

    pub fn play_card(&mut self, card: &Card, origin: &'static str, cx: &mut Context<Self>) {
        if self.card_is_current(card) {
            let playing = !self.playback.now.playing;
            self.set_playing(playing, cx);
            return;
        }
        self.start_card(card, origin, cx);
    }

    pub fn start_card(&mut self, card: &Card, origin: &'static str, cx: &mut Context<Self>) {
        if !card_plays(card) {
            return;
        }
        let options = PlayOptions::titled(&card.title);
        match card.kind {
            Kind::Track => {
                let track = card_track(card);
                if !self.refuses_track(Some(&track), None, cx) {
                    self.play(&[track], 0, cx);
                }
            }
            Kind::Episode => {
                let episode = card_track(card);
                if !self.refuses_track(Some(&episode), None, cx) {
                    self.play_context_from(&card.uri, &[episode], 0, origin, options, cx);
                }
            }
            Kind::LikedSongs => self.play_context_from(LIKED_SONGS, &[], 0, origin, options, cx),
            _ => self.play_context_from(&card.uri, &[], 0, origin, options, cx),
        }
    }

    pub fn home_fragment(&self) -> String {
        let feed = &self.discovery.feed;
        let sections = feed.current().map_or(0, |(home, _)| home.sections.len());
        format!(
            "home_facet={:?} home_sections={} home_pending={:?} home_error={}",
            feed.facet,
            sections,
            feed.pending,
            feed.error.is_some() || feed.refresh_failed,
        )
    }
}

fn describe_section(section: &Section) -> String {
    format!(
        "{:?} {} {:?} cards={} total={} next={:?}",
        section.layout,
        section.uri,
        section.title,
        section.cards.len(),
        section.total,
        section.next,
    )
}

impl OpenMusic {
    pub fn feed_debug(
        &mut self,
        verb: &str,
        rest: &str,
        directory: &Path,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        match verb {
            "more" if matches!(self.page, Page::Browse(_) | Page::Section { .. }) => {
                self.more_page_results();
            }
            "facet" => {
                let facet = match rest {
                    "" | "all" => String::new(),
                    facet => facet.to_owned(),
                };
                self.navigate(Page::Home);
                self.select_home_facet(facet);
            }
            "home-reload" => self.load_home(true),
            "browse-all" => self.load_browse_all(rest == "reload"),
            "browse" if !rest.is_empty() => self.navigate(Page::Browse(rest.to_owned())),
            "section" => {
                let (source, uri) = rest.split_once(' ').unwrap_or((rest, ""));
                if let Some(source) = SectionSource::from_name(source)
                    && !uri.trim().is_empty()
                {
                    self.navigate(Page::Section {
                        uri: uri.trim().to_owned(),
                        source,
                    });
                }
            }
            "playcard" if !rest.is_empty() => {
                if let Some(card) = self.visible_card(rest) {
                    if self.page_remembers_cards() && !self.card_is_current(&card) {
                        self.remember_card(&card);
                    }
                    self.play_card(&card, origin_for(&self.page), cx);
                }
            }
            "discovery" => {
                let _ = std::fs::write(directory.join("discovery.txt"), self.discovery_dump());
            }
            _ => return false,
        }
        true
    }

    fn page_remembers_cards(&self) -> bool {
        matches!(self.page, Page::Search(_))
    }

    fn visible_card(&self, uri: &str) -> Option<Card> {
        let feed = self.discovery.feed.feeds.values().map(|(_, home)| home);
        let home_cards = feed
            .flat_map(|home| &home.sections)
            .flat_map(|section| &section.cards);
        let page_cards = self
            .discovery
            .browse_pages
            .iter()
            .filter_map(|state| state.page.as_ref())
            .flat_map(|page| &page.sections)
            .flat_map(|section| &section.cards);
        let section_cards = self
            .discovery
            .sections
            .iter()
            .filter_map(|state| state.section.as_ref())
            .flat_map(|section| &section.cards);
        let search_cards = self
            .discovery
            .search_results()
            .into_iter()
            .flat_map(|results| {
                results
                    .top
                    .iter()
                    .map(|hit| &hit.card)
                    .chain(&results.featured)
                    .chain(results.cards.iter().flat_map(|(_, page)| &page.items))
            });
        let tab_cards = self
            .discovery
            .search_tabs
            .values()
            .filter_map(|tab| match &tab.items {
                Some(spotify_client::search::SearchPage::Cards(page)) => Some(&page.items),
                _ => None,
            })
            .flatten();
        let recent_cards = self.discovery.recent.items.iter().map(|hit| &hit.card);
        home_cards
            .chain(page_cards)
            .chain(section_cards)
            .chain(search_cards)
            .chain(tab_cards)
            .chain(recent_cards)
            .find(|card| card.uri == uri)
            .cloned()
            .or_else(|| self.artist_cards().into_iter().find(|card| card.uri == uri))
    }

    fn discovery_dump(&self) -> String {
        let feed = &self.discovery.feed;
        let mut lines = vec![format!(
            "home facet={:?} pending={:?} error={:?} refresh_failed={} cached={:?}",
            feed.facet,
            feed.pending,
            feed.error,
            feed.refresh_failed,
            feed.feeds.keys().collect::<Vec<_>>(),
        )];
        if let Some((home, dimmed)) = feed.current() {
            lines.push(format!("greeting={:?} dimmed={dimmed}", home.greeting));
            for entry in chip_entries(feed.chips(), &feed.facet) {
                let selected = if entry.selected { " selected" } else { "" };
                lines.push(format!("chip {:?} {}{selected}", entry.id, entry.label));
            }
            for section in &home.sections {
                lines.push(format!("section {}", describe_section(section)));
            }
        }
        let all = &self.discovery.browse_all;
        lines.push(format!(
            "browse_all title={:?} tiles={} pending={} error={:?}",
            all.all.as_ref().map_or("", |all| all.title.as_str()),
            all.all.as_ref().map_or(0, |all| all.tiles.len()),
            all.pending,
            all.error,
        ));
        for state in self.discovery.browse_pages.iter() {
            let page = state.page.as_ref();
            lines.push(format!(
                "browse {} title={:?} sections={} total={} next={:?} pending={:?} error={:?}",
                state.uri,
                page.map_or("", |page| page.title.as_str()),
                page.map_or(0, |page| page.sections.len()),
                page.map_or(0, |page| page.total),
                page.and_then(|page| page.next),
                state.pending,
                state.error,
            ));
            for section in page.iter().flat_map(|page| &page.sections) {
                lines.push(format!("  {}", describe_section(section)));
            }
        }
        for state in self.discovery.sections.iter() {
            lines.push(format!(
                "section-page {} {} pending={:?} error={:?} {}",
                state.source.name(),
                state.uri,
                state.pending,
                state.error,
                state
                    .section
                    .as_ref()
                    .map(describe_section)
                    .unwrap_or_default(),
            ));
        }
        lines.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use spotify_client::home::Layout;

    use super::*;

    fn chips() -> Vec<Chip> {
        vec![
            Chip {
                id: "music-chip".into(),
                label: "Music".into(),
                sub: vec![Chip {
                    id: "music-following-chip".into(),
                    label: "Following".into(),
                    sub: Vec::new(),
                }],
            },
            Chip {
                id: "podcasts-chip".into(),
                label: "Podcasts".into(),
                sub: Vec::new(),
            },
        ]
    }

    fn home(title: &str) -> Home {
        Home {
            facet: String::new(),
            greeting: "Good evening".into(),
            chips: chips(),
            sections: vec![Section {
                uri: format!("spotify:section:{title}"),
                title: title.into(),
                subtitle: String::new(),
                layout: Layout::Shelf,
                header: None,
                cards: Vec::new(),
                total: 0,
                next: None,
            }],
        }
    }

    fn shown(entries: &[ChipEntry]) -> Vec<(&str, bool)> {
        entries
            .iter()
            .map(|entry| (entry.label.as_ref(), entry.selected))
            .collect()
    }

    #[test]
    fn shows_sub_chips_next_to_their_parent() {
        assert_eq!(
            shown(&chip_entries(&chips(), "")),
            [("All", true), ("Music", false), ("Podcasts", false)]
        );
        assert_eq!(
            shown(&chip_entries(&chips(), "music-chip")),
            [
                ("All", false),
                ("Music", true),
                ("Following", false),
                ("Podcasts", false)
            ]
        );
        assert_eq!(
            shown(&chip_entries(&chips(), "music-following-chip")),
            [
                ("All", false),
                ("Music", true),
                ("Following", true),
                ("Podcasts", false)
            ]
        );
    }

    #[test]
    fn picking_the_active_chip_steps_back() {
        let chips = chips();
        assert_eq!(next_facet(&chips, "", "music-chip"), "music-chip");
        assert_eq!(next_facet(&chips, "music-chip", "music-chip"), "");
        assert_eq!(
            next_facet(&chips, "music-chip", "music-following-chip"),
            "music-following-chip"
        );
        assert_eq!(
            next_facet(&chips, "music-following-chip", "music-following-chip"),
            "music-chip"
        );
        assert_eq!(
            next_facet(&chips, "music-following-chip", "music-chip"),
            "music-chip"
        );
        assert_eq!(next_facet(&chips, "podcasts-chip", ""), "");
    }

    #[test]
    fn keeps_old_sections_on_screen_while_a_facet_loads() {
        let mut feed = HomeFeed::default();
        assert!(feed.needs_load(false));
        feed.insert(String::new(), Some(Instant::now()), home("All"));
        assert!(!feed.needs_load(false));
        assert!(feed.needs_load(true));
        feed.select("podcasts-chip".into());
        assert_eq!(feed.current(), None);
        feed.pending = Some("podcasts-chip".into());
        let (shown, dimmed) = feed.current().unwrap();
        assert_eq!(shown.sections[0].title, "All");
        assert!(dimmed);
        assert!(!feed.needs_load(false));
        feed.insert(
            "podcasts-chip".into(),
            Some(Instant::now()),
            home("Podcasts"),
        );
        feed.pending = None;
        let (shown, dimmed) = feed.current().unwrap();
        assert_eq!(shown.sections[0].title, "Podcasts");
        assert!(!dimmed);
        assert!(feed.find_section("spotify:section:All").is_some());
    }

    #[test]
    fn refreshes_a_cached_home_and_keeps_few_facets() {
        let mut feed = HomeFeed::default();
        feed.insert(String::new(), None, home("Cached"));
        assert!(feed.needs_load(false));
        assert_eq!(feed.current().unwrap().0.sections[0].title, "Cached");
        for index in 0..10 {
            feed.insert(format!("facet-{index}"), Some(Instant::now()), home("x"));
        }
        assert!(feed.feeds.len() <= KEPT_FACETS);
        assert!(feed.feeds.contains_key(""));
    }

    #[test]
    fn names_the_page_a_play_came_from() {
        assert_eq!(origin_for(&Page::Home), "home");
        assert_eq!(
            origin_for(&Page::Section {
                uri: "spotify:section:x".into(),
                source: SectionSource::Home,
            }),
            "home"
        );
        assert_eq!(
            origin_for(&Page::Search(super::super::state::SearchTab::All)),
            "search"
        );
        assert_eq!(origin_for(&Page::Browse("spotify:page:x".into())), "search");
        assert_eq!(
            origin_for(&Page::Section {
                uri: "spotify:section:x".into(),
                source: SectionSource::Browse,
            }),
            "search"
        );
        assert_eq!(
            origin_for(&Page::Artist("spotify:artist:x".into())),
            "artist"
        );
        assert_eq!(origin_for(&Page::Liked), connect::ORIGIN);
    }

    #[test]
    fn plays_music_and_podcast_cards_only() {
        let card = |kind, uri: &str| crate::ui::cards::card(uri, kind, "x", "", None);
        assert!(card_plays(&card(Kind::Album, "spotify:album:a")));
        assert!(card_plays(&card(
            Kind::LikedSongs,
            "spotify:user:u:collection"
        )));
        assert!(card_plays(&card(Kind::Show, "spotify:show:a")));
        assert!(card_plays(&card(Kind::Episode, "spotify:episode:a")));
        assert!(!card_plays(&card(Kind::Audiobook, "spotify:show:b")));
        assert!(!card_plays(&card(Kind::Genre, "spotify:page:a")));
        assert!(!card_plays(&card(Kind::User, "spotify:user:a")));
        let mut blocked = card(Kind::Playlist, "spotify:playlist:a");
        blocked.playable = false;
        assert!(!card_plays(&blocked));
        assert!(same_context("spotify:user:u:collection", LIKED_SONGS));
        assert!(!same_context("spotify:album:a", "spotify:album:b"));
    }

    #[test]
    fn a_song_card_plays_a_track_the_explicit_filter_can_refuse() {
        let mut song = crate::ui::cards::card("spotify:track:a", Kind::Track, "Creep", "x", None);
        song.explicit = true;
        let track = card_track(&song);
        assert_eq!(track.uri, "spotify:track:a");
        assert_eq!(track.title, "Creep");
        let filtering = spotify_client::account::AccountInfo {
            explicit_filtered: true,
            ..Default::default()
        };
        assert!(crate::areas::account::view::filtered(
            Some(&filtering),
            &track
        ));
    }
}
