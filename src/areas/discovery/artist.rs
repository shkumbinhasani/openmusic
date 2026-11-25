use std::{path::Path, rc::Rc};

use gpui::Context;
use spotify_client::{
    catalog::{Artist, Release},
    connect::PlayOptions,
    entity::{Card, Kind},
};

use super::home::ARTIST_ORIGIN;
use crate::{
    app::{OpenMusic, Page},
    ui::{
        cards::card,
        tracks::{RowClick, RowOptions},
    },
};

pub const POPULAR_ROWS: usize = 5;
pub const ABOUT_CHARS: usize = 900;
pub const SHELF_CARDS: usize = 4;
pub const PLAY_KEY: &str = "play-artist";
pub const FOLLOW_KEY: &str = "artist-follow";
pub const MORE_KEY: &str = "more-artist";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReleaseKind {
    Albums,
    Singles,
    Compilations,
}

impl ReleaseKind {
    pub const ALL: [Self; 3] = [Self::Albums, Self::Singles, Self::Compilations];

    pub fn label(self) -> &'static str {
        match self {
            Self::Albums => "Albums",
            Self::Singles => "Singles and EPs",
            Self::Compilations => "Compilations",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Albums => "albums",
            Self::Singles => "singles",
            Self::Compilations => "compilations",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.name() == name)
    }

    pub fn of(release: &Release) -> Option<Self> {
        match release.kind.as_str() {
            "Album" => Some(Self::Albums),
            "Single" => Some(Self::Singles),
            "Compilation" => Some(Self::Compilations),
            _ => None,
        }
    }
}

#[derive(Debug, Default, PartialEq)]
pub struct ArtistView {
    pub uri: String,
    pub more_tracks: bool,
    pub filter: Option<ReleaseKind>,
    pub all_releases: bool,
    pub all_related: bool,
    pub about_open: bool,
}

impl ArtistView {
    pub fn show(&mut self, uri: &str) {
        if self.uri != uri {
            *self = Self {
                uri: uri.to_owned(),
                ..Self::default()
            };
        }
    }

    pub fn pick(&mut self, kind: ReleaseKind) {
        self.filter = if self.filter == Some(kind) {
            None
        } else {
            Some(kind)
        };
        self.all_releases = false;
    }
}

pub fn release_card(release: &Release) -> Card {
    let subtitle = [
        release.year.map(|year| year.to_string()),
        Some(release.kind.clone()).filter(|kind| !kind.is_empty()),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" • ");
    card(
        &release.uri,
        Kind::Album,
        &release.name,
        subtitle,
        release.cover.clone(),
    )
}

pub fn release_cards(artist: &Artist, filter: Option<ReleaseKind>) -> Vec<Card> {
    artist
        .releases
        .iter()
        .filter(|release| filter.is_none_or(|kind| ReleaseKind::of(release) == Some(kind)))
        .map(release_card)
        .collect()
}

pub fn related_cards(artist: &Artist) -> Vec<Card> {
    artist
        .related
        .iter()
        .map(|related| {
            card(
                &related.uri,
                Kind::Artist,
                &related.name,
                "Artist",
                related.image.clone(),
            )
        })
        .collect()
}

pub fn release_kinds(artist: &Artist) -> Vec<ReleaseKind> {
    ReleaseKind::ALL
        .into_iter()
        .filter(|kind| {
            artist
                .releases
                .iter()
                .any(|release| ReleaseKind::of(release) == Some(*kind))
        })
        .collect()
}

pub fn popular_count(total: usize, more: bool) -> usize {
    if more { total } else { total.min(POPULAR_ROWS) }
}

pub fn about_text(biography: &str, open: bool) -> (String, bool) {
    let long = biography.chars().count() > ABOUT_CHARS;
    if open || !long {
        return (biography.to_owned(), long);
    }
    let short: String = biography.chars().take(ABOUT_CHARS).collect();
    let short = match short.rfind(char::is_whitespace) {
        Some(end) if end > ABOUT_CHARS / 2 => short[..end].trim_end().to_owned(),
        _ => short,
    };
    (format!("{short}…"), true)
}

impl OpenMusic {
    pub fn loaded_artist(&self, uri: &str) -> Option<&Artist> {
        match &self.discovery.artist {
            Some((loaded, Ok(artist))) if loaded == uri => Some(artist),
            _ => None,
        }
    }

    pub fn artist_cards(&self) -> Vec<Card> {
        match &self.discovery.artist {
            Some((_, Ok(artist))) => release_cards(artist, None)
                .into_iter()
                .chain(related_cards(artist))
                .collect(),
            _ => Vec::new(),
        }
    }

    pub fn artist_is_playing(&self, uri: &str) -> bool {
        self.context_is_current(uri) && self.playback.now.playing
    }

    pub fn play_artist(&mut self, cx: &mut Context<Self>) {
        let Page::Artist(uri) = self.page.clone() else {
            return;
        };
        if self.context_is_current(&uri) {
            let playing = !self.playback.now.playing;
            self.set_playing(playing, cx);
            return;
        }
        let tracks = self
            .loaded_artist(&uri)
            .map(|artist| artist.top_tracks.clone())
            .unwrap_or_default();
        self.play_context_from(&uri, &tracks, 0, ARTIST_ORIGIN, PlayOptions::default(), cx);
    }

    pub fn toggle_artist_follow(&mut self) {
        let Page::Artist(uri) = &self.page else {
            return;
        };
        let uri = uri.clone();
        match self.is_saved(&uri) {
            Some(followed) => self.set_saved(vec![uri], !followed),
            None => self.lookup_saved([uri]),
        }
    }

    pub fn artist_row_options(&self, uri: &str) -> RowOptions {
        let context = uri.to_owned();
        let click: RowClick = Rc::new(move |app, tracks, index, _, cx| {
            app.play_context_from(
                &context,
                tracks,
                index,
                ARTIST_ORIGIN,
                PlayOptions::default(),
                cx,
            );
        });
        RowOptions {
            context: Some(uri.to_owned()),
            show_album: false,
            on_click: Some(click),
            ..RowOptions::default()
        }
    }

    pub fn artist_fragment(&self) -> String {
        let Page::Artist(uri) = &self.page else {
            return String::new();
        };
        let Some(artist) = self.loaded_artist(uri) else {
            return format!("artist={uri} artist_loaded=false");
        };
        let view = &self.discovery.artist_view;
        format!(
            "artist={uri} artist_tracks={}/{} artist_releases={} artist_filter={} artist_related={} artist_following={:?}",
            popular_count(artist.top_tracks.len(), view.more_tracks),
            artist.top_tracks.len(),
            release_cards(artist, view.filter).len(),
            view.filter.map_or("all", ReleaseKind::name),
            artist.related.len(),
            self.is_saved(uri),
        )
    }

    pub fn artist_debug(&mut self, verb: &str, rest: &str, directory: &Path) -> bool {
        let Page::Artist(uri) = self.page.clone() else {
            return false;
        };
        let view = &mut self.discovery.artist_view;
        view.show(&uri);
        match (verb, rest) {
            ("artist-more", "tracks") => view.more_tracks = !view.more_tracks,
            ("artist-more", "releases") => view.all_releases = !view.all_releases,
            ("artist-more", "related") => view.all_related = !view.all_related,
            ("artist-more", "about") => view.about_open = !view.about_open,
            ("artist-filter", "all") => view.filter = None,
            ("artist-filter", name) => match ReleaseKind::from_name(name) {
                Some(kind) => view.pick(kind),
                None => return false,
            },
            ("artist-follow", "") => self.toggle_artist_follow(),
            ("artist-dump", "") => {
                let mut lines = vec![self.artist_fragment()];
                if let Some(artist) = self.loaded_artist(&uri) {
                    let view = &self.discovery.artist_view;
                    lines.push(format!(
                        "name={:?} image={} biography={} kinds={:?} more_tracks={} all_releases={} all_related={} about_open={}",
                        artist.name,
                        artist.image.is_some(),
                        artist.biography.as_ref().map_or(0, String::len),
                        release_kinds(artist)
                            .iter()
                            .map(|kind| kind.name())
                            .collect::<Vec<_>>(),
                        view.more_tracks,
                        view.all_releases,
                        view.all_related,
                        view.about_open,
                    ));
                    lines.extend(
                        artist
                            .top_tracks
                            .iter()
                            .map(|track| format!("track {} {}", track.uri, track.title)),
                    );
                    lines.extend(release_cards(artist, view.filter).iter().map(|card| {
                        format!("release {} {} ({})", card.uri, card.title, card.subtitle)
                    }));
                    lines.extend(
                        artist
                            .related
                            .iter()
                            .map(|related| format!("related {} {}", related.uri, related.name)),
                    );
                }
                let _ = std::fs::write(directory.join("artist.txt"), lines.join("\n"));
            }
            _ => return false,
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use spotify_client::catalog::ArtistRef;

    use super::*;

    fn release(name: &str, kind: &str, year: Option<i32>) -> Release {
        Release {
            uri: format!("spotify:album:{name}"),
            name: name.into(),
            year,
            cover: None,
            kind: kind.into(),
        }
    }

    fn artist() -> Artist {
        Artist {
            uri: "spotify:artist:4Z8W4fKeB5YxbusRsdQVPb".into(),
            name: "Radiohead".into(),
            image: None,
            biography: None,
            top_tracks: Vec::new(),
            releases: vec![
                release("a", "Album", Some(1997)),
                release("b", "Single", Some(2016)),
                release("c", "Album", None),
                release("d", "", None),
            ],
            related: vec![ArtistRef {
                uri: "spotify:artist:y".into(),
                name: "Peer".into(),
                image: None,
            }],
        }
    }

    #[test]
    fn turns_releases_and_peers_into_cards() {
        let artist = artist();
        let all = release_cards(&artist, None);
        assert_eq!(all.len(), 4);
        assert_eq!(all[0].subtitle, "1997 • Album");
        assert_eq!(all[0].kind, Kind::Album);
        assert_eq!(all[2].subtitle, "Album");
        assert_eq!(all[3].subtitle, "");
        let related = related_cards(&artist);
        assert_eq!(related[0].kind, Kind::Artist);
        assert_eq!(related[0].subtitle, "Artist");
    }

    #[test]
    fn filters_releases_by_kind() {
        let artist = artist();
        assert_eq!(
            release_kinds(&artist),
            [ReleaseKind::Albums, ReleaseKind::Singles]
        );
        let albums = release_cards(&artist, Some(ReleaseKind::Albums));
        let titles: Vec<&str> = albums.iter().map(|card| card.title.as_str()).collect();
        assert_eq!(titles, ["a", "c"]);
        assert!(release_cards(&artist, Some(ReleaseKind::Compilations)).is_empty());
        assert_eq!(
            ReleaseKind::from_name("singles"),
            Some(ReleaseKind::Singles)
        );
        assert_eq!(ReleaseKind::Singles.label(), "Singles and EPs");
    }

    #[test]
    fn picking_the_chosen_kind_again_shows_every_release() {
        let mut view = ArtistView::default();
        view.show("spotify:artist:a");
        view.all_releases = true;
        view.pick(ReleaseKind::Albums);
        assert_eq!(view.filter, Some(ReleaseKind::Albums));
        assert!(!view.all_releases);
        view.pick(ReleaseKind::Albums);
        assert_eq!(view.filter, None);
        view.more_tracks = true;
        view.show("spotify:artist:a");
        assert!(view.more_tracks);
        view.show("spotify:artist:b");
        assert_eq!(
            view,
            ArtistView {
                uri: "spotify:artist:b".into(),
                ..ArtistView::default()
            }
        );
    }

    #[test]
    fn shows_five_popular_songs_until_asked_for_more() {
        assert_eq!(popular_count(10, false), 5);
        assert_eq!(popular_count(10, true), 10);
        assert_eq!(popular_count(3, false), 3);
    }

    #[test]
    fn shortens_long_biographies_at_a_word() {
        let short = "Radiohead are an English rock band.";
        assert_eq!(about_text(short, false), (short.to_owned(), false));
        let long = "word ".repeat(400);
        let (cut, more) = about_text(&long, false);
        assert!(more);
        assert!(cut.ends_with("word…"));
        assert!(cut.chars().count() <= ABOUT_CHARS + 1);
        let (full, more) = about_text(&long, true);
        assert!(more);
        assert_eq!(full, long);
    }
}
