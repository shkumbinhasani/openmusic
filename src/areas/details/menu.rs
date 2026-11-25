use std::collections::HashMap;

use spotify_client::catalog::{Collection, Link};

use crate::{
    app::{OpenMusic, Page},
    ui::menu::{Group, MenuItem, Target, TrackTarget},
};

use super::{go, share};

pub struct Facts<'a> {
    pub page: &'a Page,
    pub artists: &'a HashMap<String, Vec<Link>>,
    pub collection: Option<&'a Collection>,
}

impl<'a> Facts<'a> {
    pub fn of(app: &'a OpenMusic) -> Self {
        Self {
            page: &app.page,
            artists: &app.details.artists,
            collection: app
                .playlists
                .collection
                .as_ref()
                .and_then(|(_, result)| result.as_ref().ok()),
        }
    }
}

pub fn items(target: &Target, facts: &Facts) -> Vec<(Group, MenuItem)> {
    match target {
        Target::Track(track) if track.track.uri.starts_with("spotify:episode:") => {
            episode(&track.track.uri, facts)
        }
        Target::Track(track) => song(track, facts),
        Target::Episode { uri } => episode(uri, facts),
        Target::Album { uri } => with_share(go::album_items(uri, facts), uri),
        Target::Artist { uri } => with_share(go::artist_items(uri), uri),
        Target::Playlist { uri } | Target::Show { uri } | Target::User { uri } => shared(uri),
        Target::Folder { .. } => Vec::new(),
    }
}

fn with_share(mut items: Vec<(Group, MenuItem)>, uri: &str) -> Vec<(Group, MenuItem)> {
    items.extend(shared(uri));
    items
}

fn song(target: &TrackTarget, facts: &Facts) -> Vec<(Group, MenuItem)> {
    if !target.track.is_track() {
        return Vec::new();
    }
    let uri = target.track.uri.clone();
    let title = target.track.title.clone();
    let mut items = go::track_items(target, facts);
    items.push((
        Group::Info,
        MenuItem::action("View credits", move |app, window, cx| {
            app.show_credits(uri.clone(), title.clone(), window, cx)
        })
        .key("i")
        .symbol("person.2"),
    ));
    with_share(items, &target.track.uri)
}

fn episode(uri: &str, facts: &Facts) -> Vec<(Group, MenuItem)> {
    let mut items = Vec::new();
    let page = Page::Episode(uri.to_owned());
    if *facts.page != page {
        items.push((
            Group::Info,
            MenuItem::action("See Episode Description", move |app, _, _| {
                app.navigate(page.clone())
            })
            .symbol("doc.text"),
        ));
    }
    items.extend(shared(uri));
    items
}

fn shared(uri: &str) -> Vec<(Group, MenuItem)> {
    share::menu_item(uri)
        .map(|item| (Group::Share, item))
        .into_iter()
        .collect()
}

pub fn describe(items: &[MenuItem], depth: usize, lines: &mut Vec<String>) {
    let indent = "  ".repeat(depth);
    for item in items {
        match item {
            MenuItem::Action {
                title,
                enabled,
                checked,
                symbol,
                key,
                subtitle,
                alternate,
                ..
            } => {
                let mut line = format!("{indent}{title}");
                if let Some(key) = key {
                    line.push_str(&format!("  {key}"));
                }
                if let Some(subtitle) = subtitle {
                    line.push_str(&format!("  — {subtitle}"));
                }
                if let Some(symbol) = symbol {
                    line.push_str(&format!("  <{symbol}>"));
                }
                if *checked {
                    line.push_str(" ✓");
                }
                if !enabled {
                    line.push_str(" [x]");
                }
                lines.push(line);
                if let Some((title, _)) = alternate {
                    lines.push(format!("{indent}(⌥) {title}"));
                }
            }
            MenuItem::Note(text) => lines.push(format!("{indent}{text} [x]")),
            MenuItem::Header(text) => lines.push(format!("{indent}# {text}")),
            MenuItem::Separator => lines.push(format!("{indent}──")),
            MenuItem::Submenu {
                title,
                symbol,
                items,
                search,
                ..
            } => {
                let mut line = format!("{indent}{title} ▸");
                if let Some(symbol) = symbol {
                    line.push_str(&format!("  <{symbol}>"));
                }
                lines.push(line);
                if let Some(placeholder) = search {
                    lines.push(format!("{indent}  [search “{placeholder}”]"));
                }
                describe(items, depth + 1, lines);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use spotify_client::{
        Track,
        catalog::{Capabilities, Collection, Link},
    };

    use super::*;
    use crate::ui::menu::{Surface, grouped};

    const SONG: &str = "spotify:track:70LcF31zb1H0PyJoS1Sx1r";
    const EPISODE: &str = "spotify:episode:0123456789abcdefghijkl";
    const RADIOHEAD: &str = "spotify:artist:4Z8W4fKeB5YxbusRsdQVPb";
    const BOWIE: &str = "spotify:artist:0oSGxfWSnnOXhD2fKuz2Gy";
    const ALBUM: &str = "spotify:album:6400dnyeDyD2mIFHfkwHXN";
    const RADIO: &str = "spotify:playlist:37i9dQZF1E8Redacted001";

    const SHARE_SONG: [&str; 4] = [
        "Share ▸  <square.and.arrow.up>",
        "  Copy link to Song  c  <link>",
        "  (⌥) Copy Spotify URI",
        "  Embed track  <chevron.left.forwardslash.chevron.right>",
    ];

    struct World {
        page: Page,
        artists: HashMap<String, Vec<Link>>,
        collection: Option<Collection>,
    }

    impl World {
        fn new() -> Self {
            Self {
                page: Page::Home,
                artists: HashMap::new(),
                collection: None,
            }
        }

        fn on(mut self, page: Page) -> Self {
            self.page = page;
            self
        }

        fn menu(&self, target: &Target) -> Vec<String> {
            let facts = Facts {
                page: &self.page,
                artists: &self.artists,
                collection: self.collection.as_ref(),
            };
            let mut lines = Vec::new();
            describe(&grouped(items(target, &facts)), 0, &mut lines);
            lines
        }
    }

    fn menu(target: &Target, page: &Page) -> Vec<String> {
        World::new().on(page.clone()).menu(target)
    }

    fn creep() -> Track {
        Track {
            title: "Creep".into(),
            artist: "Radiohead".into(),
            artist_uri: Some(RADIOHEAD.into()),
            album_uri: Some(ALBUM.into()),
            artists: vec![Link {
                uri: RADIOHEAD.into(),
                name: "Radiohead".into(),
            }],
            ..Track::placeholder(SONG)
        }
    }

    fn song_target(uri: &str, surface: Surface) -> Target {
        let mut track = Track::placeholder(uri);
        track.title = "Creep".into();
        Target::track(track, None, surface)
    }

    fn collection(uri: &str, artist: Option<&str>, edit_items: bool) -> Collection {
        Collection {
            uri: uri.into(),
            kind: "PLAYLIST".into(),
            name: "Creep Radio".into(),
            subtitle: String::new(),
            cover: None,
            artist_uri: artist.map(str::to_owned),
            tracks: Vec::new(),
            owner: None,
            description: String::new(),
            revision: None,
            format: None,
            capabilities: Capabilities {
                view: true,
                edit_items,
                ..Capabilities::default()
            },
            metadata: HashMap::new(),
        }
    }

    #[test]
    fn songs_get_navigation_credits_and_share() {
        let target = Target::track(creep(), None, Surface::Row);
        let lines = menu(&target, &Page::Home);
        let mut expected = vec![
            "Go to song radio  r  <dot.radiowaves.left.and.right>",
            "Go to artist  alt-a  <music.mic>",
            "Go to album  a  <square.stack>",
            "View credits  i  <person.2>",
            "──",
        ];
        expected.extend(SHARE_SONG);
        assert_eq!(lines, expected);
        let playing = Target::track(creep(), None, Surface::NowPlaying);
        assert_eq!(menu(&playing, &Page::Home), lines);
    }

    #[test]
    fn hides_go_to_items_on_their_own_page() {
        let target = Target::track(creep(), None, Surface::Row);
        let on_album = menu(&target, &Page::Collection(ALBUM.into()));
        assert!(on_album.iter().all(|line| !line.starts_with("Go to album")));
        assert!(on_album.contains(&"Go to artist  alt-a  <music.mic>".to_owned()));
        let on_artist = menu(&target, &Page::Artist(RADIOHEAD.into()));
        assert!(
            on_artist
                .iter()
                .all(|line| !line.starts_with("Go to artist"))
        );
        assert!(on_artist.contains(&"Go to album  a  <square.stack>".to_owned()));
    }

    #[test]
    fn lists_every_artist_in_a_submenu() {
        let mut track = creep();
        track.artists.push(Link {
            uri: BOWIE.into(),
            name: "David Bowie".into(),
        });
        let target = Target::track(track, None, Surface::Row);
        let lines = menu(&target, &Page::Home);
        let at = lines
            .iter()
            .position(|line| line == "Go to artist ▸  <music.mic>")
            .expect("submenu");
        assert_eq!(lines[at + 1..at + 3], ["  Radiohead", "  David Bowie"]);
        let lines = menu(&target, &Page::Artist(BOWIE.into()));
        let at = lines
            .iter()
            .position(|line| line == "Go to artist ▸  <music.mic>")
            .expect("submenu");
        assert_eq!(lines[at + 1], "  Radiohead");
        assert!(!lines[at + 2].starts_with("  "));
    }

    #[test]
    fn finds_artists_for_connect_tracks() {
        let mut track = creep();
        track.artists.clear();
        let target = Target::track(track.clone(), None, Surface::NowPlaying);
        let mut world = World::new();
        assert!(
            world
                .menu(&target)
                .contains(&"Go to artist  alt-a  <music.mic>".to_owned())
        );
        world.artists.insert(
            SONG.into(),
            vec![
                Link {
                    uri: RADIOHEAD.into(),
                    name: "Radiohead".into(),
                },
                Link {
                    uri: BOWIE.into(),
                    name: "David Bowie".into(),
                },
            ],
        );
        assert!(
            world
                .menu(&target)
                .contains(&"Go to artist ▸  <music.mic>".to_owned())
        );
        track.artist_uri = None;
        world.artists.clear();
        let lines = world.menu(&Target::track(track, None, Surface::NowPlaying));
        assert!(lines.iter().all(|line| !line.starts_with("Go to artist")));
    }

    #[test]
    fn local_files_and_folders_get_nothing() {
        let local = song_target("spotify:local:Artist:Album:Title:200", Surface::Row);
        assert!(menu(&local, &Page::Home).is_empty());
        let folder = Target::Folder {
            uri: "spotify:user:someone:folder:0123456789abcdef".into(),
        };
        assert!(menu(&folder, &Page::Home).is_empty());
        let liked = Target::for_uri("spotify:collection:tracks").unwrap();
        assert!(menu(&liked, &Page::Home).is_empty());
    }

    #[test]
    fn episodes_link_to_their_description() {
        let lines = menu(
            &Target::Episode {
                uri: EPISODE.into(),
            },
            &Page::Home,
        );
        assert_eq!(
            lines,
            [
                "See Episode Description  <doc.text>",
                "──",
                "Share ▸  <square.and.arrow.up>",
                "  Copy link to Episode  c  <link>",
                "  (⌥) Copy Spotify URI",
                "  Embed episode  <chevron.left.forwardslash.chevron.right>",
            ]
        );
        let playing = song_target(EPISODE, Surface::NowPlaying);
        assert_eq!(menu(&playing, &Page::Home), lines);
        let here = menu(&playing, &Page::Episode(EPISODE.into()));
        assert_eq!(here[0], "Share ▸  <square.and.arrow.up>");
    }

    #[test]
    fn albums_artists_and_playlists_get_their_own_items() {
        let radio = "Go to artist radio  r  <dot.radiowaves.left.and.right>";
        let cases = [
            (ALBUM, Some(radio), "Copy link to Album"),
            (RADIOHEAD, Some(radio), "Copy link to artist"),
            (RADIO, None, "Copy link to playlist"),
            (
                "spotify:show:0123456789abcdefghijkl",
                None,
                "Copy link to Podcast",
            ),
        ];
        for (uri, first, label) in cases {
            let lines = menu(&Target::for_uri(uri).unwrap(), &Page::Home);
            let share = match first {
                Some(first) => {
                    assert_eq!(lines[..2], [first, "──"], "{uri}");
                    2
                }
                None => 0,
            };
            assert_eq!(lines[share], "Share ▸  <square.and.arrow.up>", "{uri}");
            assert_eq!(lines[share + 1], format!("  {label}  c  <link>"), "{uri}");
            assert!(lines[share + 3].starts_with("  Embed "), "{uri}");
        }
        let user = Target::for_uri("spotify:user:someone").unwrap();
        assert_eq!(
            menu(&user, &Page::Home),
            ["Copy link to profile  c  <link>", "(⌥) Copy Spotify URI"]
        );
    }

    #[test]
    fn seeds_artist_radio_from_the_loaded_album() {
        let mut world = World::new();
        let facts = |world: &World| {
            go::album_seed(
                ALBUM,
                &Facts {
                    page: &world.page,
                    artists: &world.artists,
                    collection: world.collection.as_ref(),
                },
            )
        };
        assert_eq!(facts(&world), ALBUM);
        world.collection = Some(collection(ALBUM, Some(RADIOHEAD), false));
        assert_eq!(facts(&world), RADIOHEAD);
        world.collection = Some(collection(
            "spotify:album:1b5KESuOMQomF3wntoLAsk",
            Some(BOWIE),
            false,
        ));
        assert_eq!(facts(&world), ALBUM);
    }
}
