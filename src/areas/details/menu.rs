use crate::{
    app::{OpenMusic, Page},
    ui::menu::{Group, MenuItem, Target, TrackTarget},
};

use super::share;

pub struct Facts<'a> {
    pub page: &'a Page,
}

impl<'a> Facts<'a> {
    pub fn of(app: &'a OpenMusic) -> Self {
        Self { page: &app.page }
    }
}

pub fn items(target: &Target, facts: &Facts) -> Vec<(Group, MenuItem)> {
    match target {
        Target::Track(track) if track.track.uri.starts_with("spotify:episode:") => {
            episode(&track.track.uri, facts)
        }
        Target::Track(track) => song(track),
        Target::Episode { uri } => episode(uri, facts),
        Target::Album { uri }
        | Target::Artist { uri }
        | Target::Playlist { uri }
        | Target::Show { uri }
        | Target::User { uri } => shared(uri),
        Target::Folder { .. } => Vec::new(),
    }
}

fn song(target: &TrackTarget) -> Vec<(Group, MenuItem)> {
    if !target.track.is_track() {
        return Vec::new();
    }
    shared(&target.track.uri)
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
    use spotify_client::Track;

    use super::*;
    use crate::ui::menu::{Surface, grouped};

    const SONG: &str = "spotify:track:70LcF31zb1H0PyJoS1Sx1r";
    const EPISODE: &str = "spotify:episode:0123456789abcdefghijkl";

    fn menu(target: &Target, page: &Page) -> Vec<String> {
        let mut lines = Vec::new();
        describe(&grouped(items(target, &Facts { page })), 0, &mut lines);
        lines
    }

    fn song_target(uri: &str, surface: Surface) -> Target {
        let mut track = Track::placeholder(uri);
        track.title = "Creep".into();
        Target::track(track, None, surface)
    }

    #[test]
    fn songs_share_a_link_or_the_uri() {
        let lines = menu(&song_target(SONG, Surface::Row), &Page::Home);
        assert_eq!(
            lines,
            [
                "Share ▸  <square.and.arrow.up>",
                "  Copy link to Song  c  <link>",
                "  (⌥) Copy Spotify URI",
            ]
        );
        assert_eq!(
            menu(&song_target(SONG, Surface::NowPlaying), &Page::Home),
            lines
        );
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
            ]
        );
        let playing = song_target(EPISODE, Surface::NowPlaying);
        assert_eq!(menu(&playing, &Page::Home), lines);
        let here = menu(&playing, &Page::Episode(EPISODE.into()));
        assert_eq!(here[0], "Share ▸  <square.and.arrow.up>");
    }

    #[test]
    fn every_kind_names_its_link() {
        let cases = [
            ("spotify:album:1b5KESuOMQomF3wntoLAsk", "Copy link to Album"),
            (
                "spotify:artist:4Z8W4fKeB5YxbusRsdQVPb",
                "Copy link to artist",
            ),
            (
                "spotify:playlist:37i9dQZF1DXcBWIGoYBM5M",
                "Copy link to playlist",
            ),
            (
                "spotify:show:0123456789abcdefghijkl",
                "Copy link to Podcast",
            ),
        ];
        for (uri, label) in cases {
            let lines = menu(&Target::for_uri(uri).unwrap(), &Page::Home);
            assert_eq!(lines[0], "Share ▸  <square.and.arrow.up>", "{uri}");
            assert_eq!(lines[1], format!("  {label}  c  <link>"), "{uri}");
        }
        let user = Target::for_uri("spotify:user:someone").unwrap();
        assert_eq!(
            menu(&user, &Page::Home),
            ["Copy link to profile  c  <link>", "(⌥) Copy Spotify URI"]
        );
    }
}
