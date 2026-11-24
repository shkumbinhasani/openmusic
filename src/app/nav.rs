use spotify_client::entity;

use crate::{
    app::OpenMusic,
    areas::{
        account::{self, state::ProfileListKind},
        content, details,
        discovery::{
            self,
            state::{SearchTab, SectionSource},
        },
        library, playback, playlists,
    },
};

const HISTORY_LIMIT: usize = 30;

#[derive(Clone, Debug, PartialEq)]
pub enum Page {
    Home,
    Search(SearchTab),
    Browse(String),
    Section { uri: String, source: SectionSource },
    Artist(String),
    Collection(String),
    BlendInvitation(String),
    Track(String),
    Queue,
    Jam,
    JamInvite(String),
    Liked,
    Recent,
    Folder(String),
    Lyrics,
    Show(String),
    Episode(String),
    YourEpisodes,
    NowPlaying,
    Settings,
    Profile(String),
    ProfileList(String, ProfileListKind),
    LocalFiles,
}

impl Page {
    pub fn fills_content(&self) -> bool {
        matches!(self, Self::Lyrics | Self::NowPlaying)
    }

    pub fn describe(&self) -> String {
        match self {
            Self::Home => "home".into(),
            Self::Search(SearchTab::All) => "search".into(),
            Self::Search(tab) => format!("search {}", tab.name()),
            Self::Browse(uri) => format!("browse {uri}"),
            Self::Section { uri, source } => format!("section {} {uri}", source.name()),
            Self::Artist(uri) => format!("artist {uri}"),
            Self::Collection(uri) => format!("collection {uri}"),
            Self::BlendInvitation(id) => format!("blend {id}"),
            Self::Track(uri) => format!("track {uri}"),
            Self::Queue => "queue".into(),
            Self::Jam => "jam".into(),
            Self::JamInvite(uri) => format!("jaminvite {uri}"),
            Self::Liked => "liked".into(),
            Self::Recent => "recent".into(),
            Self::Folder(uri) => format!("folder {uri}"),
            Self::Lyrics => "lyrics".into(),
            Self::Show(uri) => format!("show {uri}"),
            Self::Episode(uri) => format!("episode {uri}"),
            Self::YourEpisodes => "yourepisodes".into(),
            Self::NowPlaying => "nowplaying".into(),
            Self::Settings => "settings".into(),
            Self::Profile(uri) => format!("profile {uri}"),
            Self::ProfileList(uri, kind) => format!("profilelist {} {uri}", kind.name()),
            Self::LocalFiles => "localfiles".into(),
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        let mut words = text.split_whitespace();
        let name = words.next()?;
        let first = words.next();
        let second = words.next();
        if words.next().is_some() {
            return None;
        }
        let uri = || match (first, second) {
            (Some(uri), None) => Some(uri.to_owned()),
            _ => None,
        };
        let none = first.is_none();
        match name {
            "home" if none => Some(Self::Home),
            "search" => match (first, second) {
                (None, _) => Some(Self::Search(SearchTab::All)),
                (Some(tab), None) => SearchTab::from_name(tab).map(Self::Search),
                _ => None,
            },
            "browse" => uri().map(Self::Browse),
            "section" => Some(Self::Section {
                source: SectionSource::from_name(first?)?,
                uri: second?.to_owned(),
            }),
            "artist" => uri().map(Self::Artist),
            "collection" => uri().map(Self::Collection),
            "blend" => uri().map(Self::BlendInvitation),
            "track" => uri().map(Self::Track),
            "queue" if none => Some(Self::Queue),
            "jam" if none => Some(Self::Jam),
            "jaminvite" => uri().map(Self::JamInvite),
            "liked" if none => Some(Self::Liked),
            "recent" if none => Some(Self::Recent),
            "folder" => uri().map(Self::Folder),
            "lyrics" if none => Some(Self::Lyrics),
            "show" => uri().map(Self::Show),
            "episode" => uri().map(Self::Episode),
            "yourepisodes" if none => Some(Self::YourEpisodes),
            "nowplaying" if none => Some(Self::NowPlaying),
            "settings" if none => Some(Self::Settings),
            "profile" => uri().map(Self::Profile),
            "profilelist" => Some(Self::ProfileList(
                second?.to_owned(),
                ProfileListKind::from_name(first?)?,
            )),
            "localfiles" if none => Some(Self::LocalFiles),
            _ => None,
        }
    }
}

pub fn page_for(uri: &str) -> Option<Page> {
    if entity::is_liked_songs(uri) {
        return Some(Page::Liked);
    }
    if uri == "spotify:collection:your-episodes"
        || (uri.starts_with("spotify:user:") && uri.ends_with(":collection:your-episodes"))
    {
        return Some(Page::YourEpisodes);
    }
    let parts: Vec<&str> = uri.split(':').collect();
    match parts.as_slice() {
        ["spotify", kind, id] if !id.is_empty() => {
            let uri = uri.to_owned();
            match *kind {
                "artist" => Some(Page::Artist(uri)),
                "album" | "playlist" => Some(Page::Collection(uri)),
                "track" => Some(Page::Track(uri)),
                "show" => Some(Page::Show(uri)),
                "episode" => Some(Page::Episode(uri)),
                "user" => Some(Page::Profile(uri)),
                "genre" | "page" => Some(Page::Browse(uri)),
                "socialsession" => Some(Page::JamInvite(uri)),
                _ => None,
            }
        }
        ["spotify", "user", _, "playlist", id] if !id.is_empty() => {
            Some(Page::Collection(format!("spotify:playlist:{id}")))
        }
        ["spotify", "user", _, "folder", id] if !id.is_empty() => {
            Some(Page::Folder(uri.to_owned()))
        }
        _ => None,
    }
}

impl OpenMusic {
    pub fn navigate(&mut self, page: Page) {
        if self.page == page {
            return;
        }
        self.load_page(&page);
        let previous = std::mem::replace(&mut self.page, page);
        push_limited(&mut self.history, previous);
        self.ahead.clear();
    }

    pub fn back(&mut self) {
        let page = self.history.pop().unwrap_or(Page::Home);
        self.load_page(&page);
        let current = std::mem::replace(&mut self.page, page);
        if current != self.page {
            push_limited(&mut self.ahead, current);
        }
    }

    pub fn forward(&mut self) {
        let Some(page) = self.ahead.pop() else {
            return;
        };
        self.load_page(&page);
        let previous = std::mem::replace(&mut self.page, page);
        push_limited(&mut self.history, previous);
    }

    fn load_page(&mut self, page: &Page) {
        match page {
            Page::Home
            | Page::Search(_)
            | Page::Browse(_)
            | Page::Section { .. }
            | Page::Artist(_) => discovery::state::load(self, page),
            Page::Collection(_) | Page::BlendInvitation(_) => playlists::state::load(self, page),
            Page::Track(_) => details::state::load(self, page),
            Page::Queue | Page::Jam | Page::JamInvite(_) => playback::state::load(self, page),
            Page::Liked | Page::Recent | Page::Folder(_) => library::state::load(self, page),
            Page::Lyrics
            | Page::Show(_)
            | Page::Episode(_)
            | Page::YourEpisodes
            | Page::NowPlaying => content::state::load(self, page),
            Page::Settings | Page::Profile(_) | Page::ProfileList(..) | Page::LocalFiles => {
                account::state::load(self, page)
            }
        }
    }
}

fn push_limited(stack: &mut Vec<Page>, page: Page) {
    stack.push(page);
    if stack.len() > HISTORY_LIMIT {
        stack.remove(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn every_page() -> Vec<Page> {
        let mut pages = vec![
            Page::Home,
            Page::Browse("spotify:page:0JQ5DAqbMKFSi39LMRT0Cy".into()),
            Page::Artist("spotify:artist:7G1GBhoKtEPnP86X2PvEYO".into()),
            Page::Collection("spotify:album:1b5KESuOMQomF3wntoLAsk".into()),
            Page::BlendInvitation("abc123".into()),
            Page::Track("spotify:track:4uLU6hMCjMI75M1A2tKUQC".into()),
            Page::Queue,
            Page::Jam,
            Page::JamInvite("spotify:socialsession:token".into()),
            Page::Liked,
            Page::Recent,
            Page::Folder("spotify:user:someone:folder:abc".into()),
            Page::Lyrics,
            Page::Show("spotify:show:5CfCWKI5pZ28U0uOzXkDHe".into()),
            Page::Episode("spotify:episode:0Q86acNRm6V9GYx55SXKwf".into()),
            Page::YourEpisodes,
            Page::NowPlaying,
            Page::Settings,
            Page::Profile("spotify:user:someone".into()),
            Page::LocalFiles,
        ];
        pages.extend(SearchTab::TABS.into_iter().map(Page::Search));
        pages.extend(
            SectionSource::SOURCES
                .into_iter()
                .map(|source| Page::Section {
                    uri: "spotify:section:0JQ5DAnM3wGh0gz1MXnu3B".into(),
                    source,
                }),
        );
        pages.extend(
            ProfileListKind::KINDS
                .into_iter()
                .map(|kind| Page::ProfileList("spotify:user:someone".into(), kind)),
        );
        pages
    }

    #[test]
    fn every_page_has_a_debug_name() {
        for page in every_page() {
            assert_eq!(
                Page::parse(&page.describe()),
                Some(page.clone()),
                "{page:?}"
            );
        }
    }

    #[test]
    fn keeps_the_old_state_names() {
        assert_eq!(Page::Home.describe(), "home");
        assert_eq!(Page::Search(SearchTab::All).describe(), "search");
        assert_eq!(Page::Lyrics.describe(), "lyrics");
        assert_eq!(
            Page::Artist("spotify:artist:x".into()).describe(),
            "artist spotify:artist:x"
        );
        assert_eq!(
            Page::Collection("spotify:album:x".into()).describe(),
            "collection spotify:album:x"
        );
    }

    #[test]
    fn refuses_bad_debug_names() {
        assert_eq!(Page::parse(""), None);
        assert_eq!(Page::parse("artist"), None);
        assert_eq!(Page::parse("home extra"), None);
        assert_eq!(Page::parse("search nothing"), None);
        assert_eq!(Page::parse("section elsewhere spotify:section:x"), None);
        assert_eq!(Page::parse("profilelist spotify:user:x"), None);
        assert_eq!(Page::parse("unknown"), None);
    }

    #[test]
    fn routes_uris_to_pages() {
        let cases = [
            (
                "spotify:artist:a",
                Some(Page::Artist("spotify:artist:a".into())),
            ),
            (
                "spotify:album:a",
                Some(Page::Collection("spotify:album:a".into())),
            ),
            (
                "spotify:playlist:a",
                Some(Page::Collection("spotify:playlist:a".into())),
            ),
            (
                "spotify:user:someone:playlist:a",
                Some(Page::Collection("spotify:playlist:a".into())),
            ),
            ("spotify:collection:tracks", Some(Page::Liked)),
            ("spotify:user:someone:collection", Some(Page::Liked)),
            ("spotify:collection:your-episodes", Some(Page::YourEpisodes)),
            ("spotify:show:a", Some(Page::Show("spotify:show:a".into()))),
            (
                "spotify:episode:a",
                Some(Page::Episode("spotify:episode:a".into())),
            ),
            (
                "spotify:track:a",
                Some(Page::Track("spotify:track:a".into())),
            ),
            (
                "spotify:user:someone",
                Some(Page::Profile("spotify:user:someone".into())),
            ),
            (
                "spotify:genre:pop",
                Some(Page::Browse("spotify:genre:pop".into())),
            ),
            (
                "spotify:page:a",
                Some(Page::Browse("spotify:page:a".into())),
            ),
            (
                "spotify:socialsession:t",
                Some(Page::JamInvite("spotify:socialsession:t".into())),
            ),
            (
                "spotify:user:someone:folder:f",
                Some(Page::Folder("spotify:user:someone:folder:f".into())),
            ),
            ("spotify:local:a:b:c:1", None),
            ("spotify:track:", None),
            ("https://open.spotify.com/track/a", None),
        ];
        for (uri, page) in cases {
            assert_eq!(page_for(uri), page, "{uri}");
        }
    }
}
