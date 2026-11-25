use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::catalog::{Link, Track, strip_tags};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Kind {
    Track,
    Album,
    Artist,
    Playlist,
    Show,
    Episode,
    Audiobook,
    Author,
    User,
    Genre,
    LikedSongs,
    Feature,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Card {
    pub uri: String,
    pub kind: Kind,
    pub title: String,
    pub subtitle: String,
    pub image: Option<String>,
    pub format: Option<String>,
    pub color: Option<u32>,
    pub reason: Option<String>,
    pub explicit: bool,
    pub playable: bool,
}

pub const CARD_IMAGE: u32 = 300;
const UNKNOWN_WIDTH: u64 = 640;
const SKIPPED: [&str; 4] = [
    "NotFound",
    "GenericError",
    "RestrictedContent",
    "UnknownType",
];

pub struct Unwrapped<'a> {
    pub typename: &'a str,
    pub data: &'a Value,
    pub uri: Option<&'a str>,
}

fn text(value: &Value) -> Option<&str> {
    value.as_str().filter(|text| !text.is_empty())
}

fn label(value: &Value) -> Option<&str> {
    text(&value["transformedLabel"]).or_else(|| text(value))
}

pub fn unwrap(value: &Value) -> Option<Unwrapped<'_>> {
    let mut current = value;
    let mut outer_uri = None;
    while current.is_object() && current.get("__typename").is_none() {
        outer_uri = text(&current["uri"]).or(outer_uri);
        current = ["item", "content", "entity"]
            .iter()
            .map(|key| &current[*key])
            .find(|inner| inner.is_object())?;
    }
    let typename = current["__typename"].as_str()?;
    let (data, wrapper_uri) = if typename.ends_with("Wrapper") {
        (&current["data"], text(&current["_uri"]))
    } else {
        (current, None)
    };
    let typename = data["__typename"].as_str().unwrap_or(typename);
    Some(Unwrapped {
        typename,
        data,
        uri: text(&data["uri"]).or(wrapper_uri).or(outer_uri),
    })
}

pub fn is_liked_songs(uri: &str) -> bool {
    uri == "spotify:collection:tracks"
        || uri == "spotify:collection"
        || (uri.starts_with("spotify:user:")
            && (uri.ends_with(":collection") || uri.ends_with(":collection:tracks")))
}

pub fn pick_image(sources: &Value, target: u32) -> Option<String> {
    let target = u64::from(target);
    sources
        .as_array()?
        .iter()
        .filter_map(|source| {
            let url = text(&source["url"])?;
            let width = source["width"]
                .as_u64()
                .or_else(|| source["maxWidth"].as_u64())
                .unwrap_or(UNKNOWN_WIDTH);
            Some((width.abs_diff(target), url))
        })
        .min_by_key(|(distance, _)| *distance)
        .map(|(_, url)| url.to_owned())
}

fn names(items: &Value, path: &[&str]) -> Vec<String> {
    items
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let name = path.iter().fold(item, |value, key| &value[*key]);
            text(name).map(str::to_owned)
        })
        .collect()
}

fn artist_names(data: &Value) -> Vec<String> {
    let mut found = names(&data["artists"]["items"], &["profile", "name"]);
    if found.is_empty() {
        found = names(&data["firstArtist"]["items"], &["profile", "name"]);
        found.extend(names(&data["otherArtists"]["items"], &["profile", "name"]));
    }
    found
}

fn links(items: &Value) -> Vec<Link> {
    items
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| {
            Some(Link {
                uri: text(&item["uri"])?.to_owned(),
                name: text(&item["profile"]["name"])?.to_owned(),
            })
        })
        .collect()
}

fn artist_links(data: &Value) -> Vec<Link> {
    let mut found = links(&data["artists"]["items"]);
    if found.is_empty() {
        found = links(&data["firstArtist"]["items"]);
        found.extend(links(&data["otherArtists"]["items"]));
    }
    found
}

fn first_artist_uri(data: &Value) -> Option<String> {
    ["artists", "firstArtist"]
        .iter()
        .find_map(|key| text(&data[*key]["items"][0]["uri"]))
        .map(str::to_owned)
}

fn duration_ms(data: &Value) -> u32 {
    ["duration", "trackDuration"]
        .iter()
        .find_map(|key| data[*key]["totalMilliseconds"].as_u64())
        .unwrap_or_default()
        .min(u64::from(u32::MAX)) as u32
}

pub fn explicit(data: &Value) -> bool {
    data["contentRating"]["label"] == "EXPLICIT"
}

pub fn playable(data: &Value) -> bool {
    data["playability"]["playable"].as_bool().unwrap_or(true)
}

pub fn color(hex: &str) -> Option<u32> {
    let digits = hex.trim().trim_start_matches('#');
    if digits.len() != 6 {
        return None;
    }
    u32::from_str_radix(digits, 16).ok()
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

pub fn short_date(iso: &str) -> Option<String> {
    let mut parts = iso.get(..10)?.split('-');
    let year: u32 = parts.next()?.parse().ok()?;
    let month: usize = parts.next()?.parse().ok()?;
    let day: u32 = parts.next()?.parse().ok()?;
    let month = MONTHS.get(month.checked_sub(1)?)?;
    Some(format!("{month} {day}, {year}"))
}

pub fn long_duration(milliseconds: u32) -> Option<String> {
    let minutes = milliseconds / 60_000;
    if milliseconds == 0 {
        return None;
    }
    Some(match (minutes / 60, minutes % 60) {
        (0, 0) => format!("{} sec", milliseconds / 1000),
        (0, minutes) => format!("{minutes} min"),
        (hours, minutes) => format!("{hours} hr {minutes} min"),
    })
}

fn joined(parts: impl IntoIterator<Item = Option<String>>, separator: &str) -> String {
    parts
        .into_iter()
        .flatten()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(separator)
}

fn kind_from_uri(uri: &str) -> Option<Kind> {
    if is_liked_songs(uri) {
        return Some(Kind::LikedSongs);
    }
    let kind = uri.strip_prefix("spotify:")?.split(':').next()?;
    Some(match kind {
        "track" => Kind::Track,
        "album" => Kind::Album,
        "artist" => Kind::Artist,
        "playlist" => Kind::Playlist,
        "show" => Kind::Show,
        "episode" | "chapter" => Kind::Episode,
        "author" => Kind::Author,
        "user" => Kind::User,
        "genre" | "page" => Kind::Genre,
        _ => return None,
    })
}

fn entity_kind(data: &Value, uri: &str) -> Option<Kind> {
    let kind = match data["entityTypeTrait"]["type"].as_str() {
        Some("ENTITY_TYPE_TRACK") => Kind::Track,
        Some("ENTITY_TYPE_ALBUM") => Kind::Album,
        Some("ENTITY_TYPE_ARTIST") => Kind::Artist,
        Some("ENTITY_TYPE_PLAYLIST") => Kind::Playlist,
        Some("ENTITY_TYPE_SHOW") => Kind::Show,
        Some("ENTITY_TYPE_EPISODE") | Some("ENTITY_TYPE_CHAPTER") => Kind::Episode,
        Some("ENTITY_TYPE_AUDIOBOOK") => Kind::Audiobook,
        _ => kind_from_uri(uri)?,
    };
    Some(kind)
}

fn blank(uri: String, kind: Kind, title: String) -> Card {
    Card {
        uri,
        kind,
        title,
        subtitle: String::new(),
        image: None,
        format: None,
        color: None,
        reason: None,
        explicit: false,
        playable: true,
    }
}

pub fn card(value: &Value) -> Option<Card> {
    let found = unwrap(value)?;
    let data = found.data;
    let uri = found.uri.unwrap_or_default().to_owned();
    if found.typename == "UnknownTypeWrapper" || found.typename == "UnknownType" {
        return is_liked_songs(&uri).then(|| blank(uri, Kind::LikedSongs, "Liked Songs".into()));
    }
    if SKIPPED.contains(&found.typename) {
        return None;
    }
    let image = |sources: &Value| pick_image(sources, CARD_IMAGE);
    let mut card = match found.typename {
        "Track" => {
            let mut card = blank(uri, Kind::Track, text(&data["name"])?.to_owned());
            card.subtitle = artist_names(data).join(", ");
            card.image = image(&data["albumOfTrack"]["coverArt"]["sources"]);
            card
        }
        "Album" => {
            let mut card = blank(uri, Kind::Album, text(&data["name"])?.to_owned());
            let artists = artist_names(data).join(", ");
            card.subtitle = match data["date"]["year"].as_i64() {
                Some(year) => joined([Some(year.to_string()), Some(artists)], " • "),
                None => artists,
            };
            card.image = image(&data["coverArt"]["sources"]);
            card
        }
        "Artist" => {
            let mut card = blank(
                uri,
                Kind::Artist,
                text(&data["profile"]["name"])?.to_owned(),
            );
            card.subtitle = "Artist".into();
            card.image = image(&data["visuals"]["avatarImage"]["sources"]);
            card
        }
        "Playlist" => {
            let mut card = blank(uri, Kind::Playlist, text(&data["name"])?.to_owned());
            let description = strip_tags(data["description"].as_str().unwrap_or_default());
            card.subtitle = if description.is_empty() {
                text(&data["ownerV2"]["data"]["name"])
                    .map(|owner| format!("By {owner}"))
                    .unwrap_or_default()
            } else {
                description
            };
            card.image = image(&data["images"]["items"][0]["sources"]);
            card.format = text(&data["format"]).map(str::to_owned);
            card
        }
        "Podcast" => {
            let mut card = blank(uri, Kind::Show, text(&data["name"])?.to_owned());
            card.subtitle = text(&data["publisher"]["name"])
                .unwrap_or_default()
                .to_owned();
            card.image = image(&data["coverArt"]["sources"]);
            card
        }
        "Episode" | "Chapter" => {
            let mut card = blank(uri, Kind::Episode, text(&data["name"])?.to_owned());
            card.subtitle = joined(
                [
                    text(&data["releaseDate"]["isoString"]).and_then(short_date),
                    long_duration(duration_ms(data)),
                ],
                " • ",
            );
            if card.subtitle.is_empty() {
                card.subtitle = text(&data["podcastV2"]["data"]["name"])
                    .unwrap_or_default()
                    .to_owned();
            }
            card.image = image(&data["coverArt"]["sources"]);
            card
        }
        "Audiobook" => {
            let mut card = blank(uri, Kind::Audiobook, text(&data["name"])?.to_owned());
            card.subtitle = names(&data["authorsV2"], &["name"]).join(", ");
            card.image = image(&data["coverArt"]["sources"]);
            card
        }
        "AuthorV2" | "Author" => {
            let mut card = blank(uri, Kind::Author, text(&data["name"])?.to_owned());
            card.subtitle = "Author".into();
            card.image = image(&data["avatar"]["sources"]);
            card
        }
        "User" => {
            let title = text(&data["displayName"])
                .or_else(|| text(&data["name"]))
                .or_else(|| text(&data["username"]))?;
            let mut card = blank(uri, Kind::User, title.to_owned());
            card.subtitle = "Profile".into();
            card.image = image(&data["avatar"]["sources"]);
            card
        }
        "Genre" => {
            let mut card = blank(uri, Kind::Genre, text(&data["name"])?.to_owned());
            card.image = image(&data["image"]["sources"]);
            card
        }
        "BrowseSectionContainer" => {
            let tile = &data["data"]["cardRepresentation"];
            let mut card = blank(uri, Kind::Genre, label(&tile["title"])?.to_owned());
            card.color = text(&tile["backgroundColor"]["hex"]).and_then(color);
            card.image = image(&tile["artwork"]["sources"]);
            card
        }
        "BrowseClientFeature" => {
            let uri = text(&data["featureUri"]).map_or(uri, str::to_owned);
            let mut card = blank(uri, Kind::Feature, label(&data["title"])?.to_owned());
            card.color = text(&data["backgroundColor"]["hex"]).and_then(color);
            card.image = image(&data["artwork"]["sources"]);
            card
        }
        "Entity" => {
            let identity = &data["identityTrait"];
            let kind = entity_kind(data, &uri)?;
            let mut card = blank(uri, kind, text(&identity["name"])?.to_owned());
            card.subtitle = names(&identity["contributors"]["items"], &["name"]).join(", ");
            card.image =
                image(&data["visualIdentityTrait"]["squareCoverImage"]["image"]["data"]["sources"]);
            card
        }
        _ => return None,
    };
    if card.uri.is_empty() {
        return None;
    }
    card.explicit = explicit(data);
    card.playable = playable(data);
    Some(card)
}

pub fn track(value: &Value) -> Option<Track> {
    let found = unwrap(value)?;
    let data = found.data;
    let uri = found.uri?.to_owned();
    let title = text(&data["name"])?.to_owned();
    match found.typename {
        "Track" => {
            let album = &data["albumOfTrack"];
            Some(Track {
                title,
                artist: artist_names(data).join(", "),
                album: text(&album["name"]).unwrap_or_default().to_owned(),
                duration_ms: duration_ms(data),
                cover: pick_image(&album["coverArt"]["sources"], CARD_IMAGE),
                artist_uri: first_artist_uri(data),
                album_uri: text(&album["uri"]).map(str::to_owned),
                artists: artist_links(data),
                explicit: explicit(data),
                uri,
                ..Track::default()
            })
        }
        "Episode" => {
            let show = &data["podcastV2"]["data"];
            let show_link = text(&show["uri"])
                .zip(text(&show["name"]))
                .map(|(uri, name)| Link {
                    uri: uri.to_owned(),
                    name: name.to_owned(),
                });
            Some(Track {
                title,
                artist: text(&show["name"]).unwrap_or_default().to_owned(),
                duration_ms: duration_ms(data),
                cover: pick_image(&data["coverArt"]["sources"], CARD_IMAGE),
                artists: show_link.into_iter().collect(),
                explicit: explicit(data),
                uri,
                ..Track::default()
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn sources(url: &str) -> Value {
        json!({"sources": [
            {"url": format!("{url}-64"), "width": 64, "height": 64},
            {"url": format!("{url}-300"), "width": 300, "height": 300},
            {"url": format!("{url}-640"), "width": 640, "height": 640}
        ]})
    }

    fn track_wrapper() -> Value {
        json!({"__typename": "TrackResponseWrapper", "data": {
            "__typename": "Track",
            "name": "Creep",
            "uri": "spotify:track:70LcF31zb1H0PyJoS1Sx1r",
            "albumOfTrack": {
                "name": "Pablo Honey",
                "uri": "spotify:album:3gBVdu4a1MMJVMy6vwPEb8",
                "coverArt": sources("https://i.scdn.co/image/creep")
            },
            "artists": {"items": [
                {"profile": {"name": "Radiohead"}, "uri": "spotify:artist:4Z8W4fKeB5YxbusRsdQVPb"}
            ]},
            "contentRating": {"label": "EXPLICIT"},
            "duration": {"totalMilliseconds": 238640},
            "playability": {"playable": true, "reason": "PLAYABLE"}
        }})
    }

    #[test]
    fn picks_the_nearest_image() {
        let picked = pick_image(&sources("a")["sources"], 300);
        assert_eq!(picked.as_deref(), Some("a-300"));
        let picked = pick_image(&sources("a")["sources"], 600);
        assert_eq!(picked.as_deref(), Some("a-640"));
        let unknown = json!([{"url": "mosaic", "width": null, "height": null}]);
        assert_eq!(pick_image(&unknown, 300).as_deref(), Some("mosaic"));
        let traits = json!([
            {"url": "small", "maxWidth": 64, "maxHeight": 64},
            {"url": "medium", "maxWidth": 300, "maxHeight": 300}
        ]);
        assert_eq!(pick_image(&traits, 300).as_deref(), Some("medium"));
        assert_eq!(pick_image(&json!([]), 300), None);
    }

    #[test]
    fn maps_a_top_result_track() {
        let hit = json!({"item": track_wrapper(), "matchedFields": []});
        let card = card(&hit).unwrap();
        assert_eq!(card.uri, "spotify:track:70LcF31zb1H0PyJoS1Sx1r");
        assert_eq!(card.kind, Kind::Track);
        assert_eq!(card.title, "Creep");
        assert_eq!(card.subtitle, "Radiohead");
        assert_eq!(
            card.image.as_deref(),
            Some("https://i.scdn.co/image/creep-300")
        );
        assert!(card.explicit);
        assert!(card.playable);

        let track = track(&hit).unwrap();
        assert_eq!(
            track,
            Track {
                uri: "spotify:track:70LcF31zb1H0PyJoS1Sx1r".into(),
                title: "Creep".into(),
                artist: "Radiohead".into(),
                album: "Pablo Honey".into(),
                duration_ms: 238640,
                cover: Some("https://i.scdn.co/image/creep-300".into()),
                artist_uri: Some("spotify:artist:4Z8W4fKeB5YxbusRsdQVPb".into()),
                album_uri: Some("spotify:album:3gBVdu4a1MMJVMy6vwPEb8".into()),
                artists: vec![Link {
                    uri: "spotify:artist:4Z8W4fKeB5YxbusRsdQVPb".into(),
                    name: "Radiohead".into(),
                }],
                explicit: true,
                ..Track::default()
            }
        );
    }

    #[test]
    fn maps_track_pages_with_first_and_other_artists() {
        let value = json!({"__typename": "Track", "name": "Counting Stars",
            "uri": "spotify:track:2tpWsVSb9UEmDRxAl1zhX1",
            "trackDuration": {"totalMilliseconds": 257265},
            "albumOfTrack": {"name": "Native", "uri": "spotify:album:20lOt6G8MHv8ZO7ViOmiP7"},
            "firstArtist": {"items": [{"uri": "spotify:artist:5Pwc4xIPtQLFEnJriah9YJ", "profile": {"name": "OneRepublic"}}]},
            "otherArtists": {"items": [{"uri": "spotify:artist:x", "profile": {"name": "Guest"}}]}
        });
        let track = track(&value).unwrap();
        assert_eq!(track.artist, "OneRepublic, Guest");
        assert_eq!(track.duration_ms, 257265);
        assert_eq!(
            track.artist_uri.as_deref(),
            Some("spotify:artist:5Pwc4xIPtQLFEnJriah9YJ")
        );
        let artists: Vec<(&str, &str)> = track
            .artists
            .iter()
            .map(|link| (link.uri.as_str(), link.name.as_str()))
            .collect();
        assert_eq!(
            artists,
            [
                ("spotify:artist:5Pwc4xIPtQLFEnJriah9YJ", "OneRepublic"),
                ("spotify:artist:x", "Guest")
            ]
        );
        assert!(!track.explicit);
    }

    #[test]
    fn maps_search_albums_and_home_albums() {
        let search = json!({"__typename": "AlbumResponseWrapper", "data": {
            "__typename": "Album", "name": "OK Computer", "type": "ALBUM",
            "date": {"year": 1997},
            "artists": {"items": [{"profile": {"name": "Radiohead"}, "uri": "spotify:artist:4Z8W4fKeB5YxbusRsdQVPb"}]},
            "coverArt": sources("ok"),
            "uri": "spotify:album:6dVIqQ8qmQ5GBnJ9shOYGE"
        }});
        let card = card(&search).unwrap();
        assert_eq!(card.kind, Kind::Album);
        assert_eq!(card.subtitle, "1997 • Radiohead");
        assert_eq!(card.image.as_deref(), Some("ok-300"));

        let home = json!({"uri": "spotify:album:6dVIqQ8qmQ5GBnJ9shOYGE", "data": null,
        "content": {"__typename": "AlbumResponseWrapper", "data": {
            "__typename": "Album", "name": "OK Computer", "albumType": "ALBUM",
            "artists": {"items": [{"profile": {"name": "Radiohead"}}]},
            "coverArt": sources("ok")
        }}});
        let card = super::card(&home).unwrap();
        assert_eq!(card.uri, "spotify:album:6dVIqQ8qmQ5GBnJ9shOYGE");
        assert_eq!(card.subtitle, "Radiohead");
    }

    #[test]
    fn maps_playlists_with_and_without_descriptions() {
        let radio = json!({"__typename": "PlaylistResponseWrapper", "data": {
            "__typename": "Playlist", "name": "Radiohead Radio", "format": "inspiredby-mix",
            "description": "",
            "uri": "spotify:playlist:37i9dQZF1E4Redacted002",
            "images": {"items": [{"sources": [{"url": "https://pickasso.spotifycdn.com/x", "width": null, "height": null}]}]},
            "ownerV2": {"data": {"name": "Spotify", "uri": "spotify:user:spotify"}}
        }});
        let card = card(&radio).unwrap();
        assert_eq!(card.kind, Kind::Playlist);
        assert_eq!(card.subtitle, "By Spotify");
        assert_eq!(card.format.as_deref(), Some("inspiredby-mix"));
        assert_eq!(
            card.image.as_deref(),
            Some("https://pickasso.spotifycdn.com/x")
        );

        let editorial = json!({"__typename": "PlaylistResponseWrapper", "data": {
            "__typename": "Playlist", "name": "Rock Classics", "format": "",
            "description": "Rock legends &amp; <a href=\"spotify:genre:x\">epic</a> songs",
            "uri": "spotify:playlist:37i9dQZF1DWXRqgorJj26U"
        }});
        let card = super::card(&editorial).unwrap();
        assert_eq!(card.subtitle, "Rock legends & epic songs");
        assert_eq!(card.format, None);
        assert_eq!(card.image, None);
    }

    #[test]
    fn maps_artists_shows_episodes_audiobooks_users_and_genres() {
        let artist = json!({"__typename": "ArtistResponseWrapper", "data": {
            "__typename": "Artist", "profile": {"name": "Radiohead"},
            "uri": "spotify:artist:4Z8W4fKeB5YxbusRsdQVPb",
            "visuals": {"avatarImage": sources("avatar")}
        }});
        let card = card(&artist).unwrap();
        assert_eq!(
            (card.kind, card.subtitle.as_str()),
            (Kind::Artist, "Artist")
        );
        assert_eq!(card.image.as_deref(), Some("avatar-300"));

        let show = json!({"__typename": "PodcastResponseWrapper", "data": {
            "__typename": "Podcast", "name": "Show", "publisher": {"name": "Publisher"},
            "uri": "spotify:show:5CfCWKI5pZ28U0uOzXkDHe", "coverArt": sources("show")
        }});
        let card = super::card(&show).unwrap();
        assert_eq!(
            (card.kind, card.subtitle.as_str()),
            (Kind::Show, "Publisher")
        );

        let episode = json!({"__typename": "EpisodeOrChapterResponseWrapper", "data": {
            "__typename": "Episode", "name": "Episode",
            "duration": {"totalMilliseconds": 3_611_237},
            "releaseDate": {"isoString": "2026-01-19T20:26:00Z", "precision": "MINUTE"},
            "podcastV2": {"data": {"name": "Show", "uri": "spotify:show:5CfCWKI5pZ28U0uOzXkDHe"}},
            "uri": "spotify:episode:0Q86acNRm6V9GYx55SXKwf",
            "coverArt": sources("episode"),
            "playability": {"playable": false}
        }});
        let card = super::card(&episode).unwrap();
        assert_eq!(card.kind, Kind::Episode);
        assert_eq!(card.subtitle, "Jan 19, 2026 • 1 hr 0 min");
        assert!(!card.playable);
        let track = track(&episode).unwrap();
        assert_eq!(
            (track.artist.as_str(), track.duration_ms),
            ("Show", 3_611_237)
        );
        assert_eq!(
            track.artists,
            [Link {
                uri: "spotify:show:5CfCWKI5pZ28U0uOzXkDHe".into(),
                name: "Show".into(),
            }]
        );

        let book = json!({"__typename": "AudiobookResponseWrapper", "data": {
            "__typename": "Audiobook", "name": "Book",
            "authorsV2": [{"name": "First", "uri": "spotify:author:a"}, {"name": "Second"}],
            "uri": "spotify:show:7iHfbu1YPACw6oZPAFJtqe"
        }});
        let card = super::card(&book).unwrap();
        assert_eq!(
            (card.kind, card.subtitle.as_str()),
            (Kind::Audiobook, "First, Second")
        );

        let user = json!({"__typename": "UserResponseWrapper", "data": {
            "__typename": "User", "displayName": "Someone", "username": "someone",
            "uri": "spotify:user:someone", "avatar": {"sources": []}
        }});
        let card = super::card(&user).unwrap();
        assert_eq!((card.kind, card.title.as_str()), (Kind::User, "Someone"));
        assert_eq!(card.subtitle, "Profile");

        let genre = json!({"__typename": "GenreResponseWrapper", "data": {
            "__typename": "Genre", "name": "RADAR", "uri": "spotify:genre:0JQ5DAqbMKFOOxftoKZxod",
            "image": {"sources": [{"url": "https://t.scdn.co/images/radar", "width": 274, "height": 274}]}
        }});
        let card = super::card(&genre).unwrap();
        assert_eq!((card.kind, card.subtitle.as_str()), (Kind::Genre, ""));
        assert_eq!(
            card.image.as_deref(),
            Some("https://t.scdn.co/images/radar")
        );
    }

    #[test]
    fn maps_browse_tiles() {
        let tile = json!({"uri": "spotify:page:0JQ5DAqbMKFSi39LMRT0Cy", "content": {
            "__typename": "BrowseSectionContainerWrapper", "data": {
                "__typename": "BrowseSectionContainer", "data": {"cardRepresentation": {
                    "title": {"transformedLabel": "Music"},
                    "backgroundColor": {"hex": "#dc148c"},
                    "artwork": {"sources": [{"url": "https://i.scdn.co/image/music", "width": 300, "height": 300}]}
                }}
            }
        }});
        let card = card(&tile).unwrap();
        assert_eq!(card.uri, "spotify:page:0JQ5DAqbMKFSi39LMRT0Cy");
        assert_eq!((card.kind, card.title.as_str()), (Kind::Genre, "Music"));
        assert_eq!(card.color, Some(0xdc148c));

        let feature = json!({"uri": "spotify:xlink:0JQ5DAozXW0GUBAKjHsifL", "content": {
            "__typename": "BrowseXlinkResponseWrapper", "data": {
                "__typename": "BrowseClientFeature", "featureUri": "spotify:concerts",
                "title": {"transformedLabel": "Live Events"}, "backgroundColor": {"hex": "#8400e7"}
            }
        }});
        let card = super::card(&feature).unwrap();
        assert_eq!(
            (card.kind, card.uri.as_str()),
            (Kind::Feature, "spotify:concerts")
        );
    }

    #[test]
    fn maps_recents_entities() {
        let item = json!({"uid": "b0a4", "entity": {
            "__typename": "EntityResponseWrapper", "_uri": "spotify:album:1b5KESuOMQomF3wntoLAsk",
            "data": {"__typename": "Entity",
                "entityTypeTrait": {"type": "ENTITY_TYPE_ALBUM"},
                "identityTrait": {"name": "Single", "type": "Single",
                    "contributors": {"items": [{"name": "One", "uri": "spotify:artist:a"}, {"name": "Two"}], "totalCount": 2}},
                "visualIdentityTrait": {"squareCoverImage": {"image": {"data": {"__typename": "ImageV2",
                    "sources": [{"imageFormat": "JPEG", "maxHeight": 300, "maxWidth": 300, "url": "https://image-cdn-ak.spotifycdn.com/image/x"}]}}}}
            }
        }});
        let card = card(&item).unwrap();
        assert_eq!(card.uri, "spotify:album:1b5KESuOMQomF3wntoLAsk");
        assert_eq!(card.kind, Kind::Album);
        assert_eq!(card.subtitle, "One, Two");
        assert_eq!(
            card.image.as_deref(),
            Some("https://image-cdn-ak.spotifycdn.com/image/x")
        );
    }

    #[test]
    fn skips_missing_and_unknown_entities() {
        for typename in [
            "NotFound",
            "GenericError",
            "RestrictedContent",
            "UnknownType",
        ] {
            let value = json!({"uri": "spotify:playlist:37i9dQZF1E8Redacted001", "content": {
                "__typename": "PlaylistResponseWrapper",
                "data": {"__typename": typename, "message": "gone"}
            }});
            assert_eq!(card(&value), None, "{typename}");
        }
        assert_eq!(card(&json!({"__typename": "UnknownTypeWrapper"})), None);
        assert_eq!(card(&json!({"__typename": "SearchSectionEntity"})), None);
        assert_eq!(card(&json!({"item": null})), None);
        assert_eq!(card(&json!("text")), None);
        assert_eq!(
            track(
                &json!({"__typename": "ArtistResponseWrapper", "data": {"__typename": "Artist", "name": "x", "uri": "spotify:artist:x"}})
            ),
            None
        );
    }

    #[test]
    fn maps_liked_songs_shortcuts() {
        let value =
            json!({"__typename": "UnknownTypeWrapper", "_uri": "spotify:user:someone:collection"});
        let card = card(&value).unwrap();
        assert_eq!(card.kind, Kind::LikedSongs);
        assert_eq!(card.title, "Liked Songs");
        assert_eq!(card.uri, "spotify:user:someone:collection");
    }

    #[test]
    fn formats_episode_details() {
        assert_eq!(
            short_date("2026-01-19T20:26:00Z").as_deref(),
            Some("Jan 19, 2026")
        );
        assert_eq!(short_date("2026-13-01"), None);
        assert_eq!(long_duration(237_087).as_deref(), Some("3 min"));
        assert_eq!(long_duration(45_000).as_deref(), Some("45 sec"));
        assert_eq!(long_duration(5_400_000).as_deref(), Some("1 hr 30 min"));
        assert_eq!(long_duration(0), None);
        assert_eq!(color("#AF2896"), Some(0xaf2896));
        assert_eq!(color("red"), None);
    }
}
