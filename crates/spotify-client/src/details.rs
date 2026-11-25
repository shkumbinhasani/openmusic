use std::collections::{HashMap, HashSet};

use anyhow::{Result, bail};
use serde_json::{Value, json};

use crate::{
    api::Api,
    entity,
    pathfinder::{self, Operation},
};

pub const TRACK_CREDITS: Operation = Operation::query(
    "queryTrackCreditsGroupedModal",
    "5c109593bca8ea4cb28104e178820ff035418fb4694d34d10c89cc755fe64694",
);

const EXTRACTED_COLOR: &str = "b2d6d99fb6237952dfa6638385f9c6085f1b1fb9d0468753ca6ad98ff45adc6f";

pub const CONTRIBUTORS_LIMIT: u32 = 100;

pub const OTHER_GROUP: &str = "Other";

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Credits {
    pub uri: String,
    pub track: String,
    pub groups: Vec<CreditGroup>,
    pub sources: Vec<String>,
}

impl Credits {
    pub fn linked_artists(&self) -> Vec<String> {
        self.groups
            .iter()
            .flat_map(|group| &group.people)
            .filter_map(|person| person.uri.clone())
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct CreditGroup {
    pub name: String,
    pub people: Vec<Contributor>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Contributor {
    pub name: String,
    pub roles: Vec<String>,
    pub uri: Option<String>,
    pub url: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Unavailable;

impl std::fmt::Display for Unavailable {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "Credits aren’t available for this song.")
    }
}

impl std::error::Error for Unavailable {}

pub fn credits_variables(uri: &str) -> Value {
    json!({
        "trackUri": uri,
        "contributorsLimit": CONTRIBUTORS_LIMIT,
        "contributorsOffset": 0,
    })
}

pub async fn credits(api: &Api, uri: &str) -> Result<Credits> {
    let data = pathfinder::query(api, TRACK_CREDITS, credits_variables(uri)).await?;
    parse_credits(uri, &data)
}

pub fn parse_credits(uri: &str, data: &Value) -> Result<Credits> {
    let track = &data["trackUnion"];
    if track["__typename"].as_str() != Some("Track") {
        bail!(Unavailable);
    }
    let main: HashSet<String> = track["artists"]["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|artist| artist["uri"].as_str())
        .map(str::to_owned)
        .collect();
    let credits = &track["creditsTrait"];
    let contributors: Vec<Value> = credits["contributors"]["items"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let sources = credits["sources"]["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|source| source["name"].as_str())
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect();
    Ok(Credits {
        uri: uri.to_owned(),
        track: track["name"].as_str().unwrap_or_default().to_owned(),
        groups: group(&contributors, &main),
        sources,
    })
}

fn text(value: &Value) -> Option<String> {
    value
        .as_str()
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

fn contributor_uri(row: &Value) -> Option<String> {
    text(&row["uri"]).or_else(|| text(&row["reference"]["uri"]))
}

fn contributor_url(row: &Value) -> Option<String> {
    text(&row["url"]).or_else(|| text(&row["reference"]["url"]))
}

pub fn group(contributors: &[Value], main: &HashSet<String>) -> Vec<CreditGroup> {
    let mut order: Vec<String> = Vec::new();
    let mut rows: HashMap<String, Vec<&Value>> = HashMap::new();
    for row in contributors {
        let name = row["roleGroup"]["name"]
            .as_str()
            .unwrap_or(OTHER_GROUP)
            .to_owned();
        if !rows.contains_key(&name) {
            order.push(name.clone());
        }
        rows.entry(name).or_default().push(row);
    }
    order
        .into_iter()
        .enumerate()
        .map(|(index, name)| {
            let people = merge(&rows[&name], index == 0, main);
            CreditGroup { name, people }
        })
        .collect()
}

fn merge(rows: &[&Value], links: bool, main: &HashSet<String>) -> Vec<Contributor> {
    let linkable =
        |uri: &Option<String>| links && uri.as_ref().is_some_and(|uri| main.contains(uri));
    let mut people: Vec<Contributor> = Vec::new();
    for row in rows {
        let Some(name) = text(&row["name"]) else {
            continue;
        };
        let role = text(&row["role"]);
        let uri = contributor_uri(row);
        let url = contributor_url(row);
        if let Some(person) = people.iter_mut().find(|person| person.name == name) {
            let Some(role) = role else {
                continue;
            };
            person.roles.push(role);
            if linkable(&uri) && person.uri.is_none() {
                person.uri = uri;
            }
            if person.uri.is_none() && person.url.is_none() {
                person.url = url;
            }
            continue;
        }
        let uri = linkable(&uri).then_some(uri).flatten();
        let url = if uri.is_some() { None } else { url };
        people.push(Contributor {
            name,
            roles: role.into_iter().collect(),
            uri,
            url,
        });
    }
    people
}

pub fn cover_color_operation(uri: &str) -> Option<Operation> {
    let name = match uri.split(':').nth(1)? {
        "track" => "fetchExtractedColorForTrackEntity",
        "album" => "fetchExtractedColorForAlbumEntity",
        "artist" => "fetchExtractedColorForArtistEntity",
        "playlist" => "fetchExtractedColorForPlaylistEntity",
        "episode" => "fetchExtractedColorForEpisodeEntity",
        "show" => "fetchExtractedColorForPodcastEntity",
        _ => return None,
    };
    Some(Operation::query(name, EXTRACTED_COLOR))
}

pub async fn cover_color(api: &Api, uri: &str) -> Result<Option<u32>> {
    let Some(operation) = cover_color_operation(uri) else {
        return Ok(None);
    };
    let data = pathfinder::query(api, operation, json!({ "uri": uri })).await?;
    Ok(parse_cover_color(&data))
}

pub fn parse_cover_color(data: &Value) -> Option<u32> {
    let image = [
        &data["albumUnion"]["coverArt"],
        &data["artistUnion"]["visuals"]["avatarImage"],
        &data["episodeUnionV2"]["coverArt"],
        &data["playlistV2"]["images"]["items"][0],
        &data["podcastUnionV2"]["coverArt"],
        &data["trackUnion"]["albumOfTrack"]["coverArt"],
    ]
    .into_iter()
    .find(|image| image.is_object())?;
    entity::color(image["extractedColors"]["colorDark"]["hex"].as_str()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    const RADIOHEAD: &str = "spotify:artist:4Z8W4fKeB5YxbusRsdQVPb";
    const HAMMOND: &str = "spotify:artist:34E3csCxpXunPGEkOVVX2g";

    fn person(name: &str, role: &str, group: &str, uri: Option<&str>) -> Value {
        json!({
            "name": name,
            "role": role,
            "roleGroup": {"name": group},
            "uri": uri,
            "url": null,
            "aiAttribution": {"hasAiAttribution": false},
        })
    }

    fn creep() -> Value {
        json!({
            "trackUnion": {
                "__typename": "Track",
                "name": "Creep",
                "artists": {"items": [{"uri": RADIOHEAD}]},
                "creditsTrait": {
                    "contributors": {"items": [
                        person("Radiohead", "Main Artist", "Artist", Some(RADIOHEAD)),
                        person("Albert Hammond", "Composer", "Composition & Lyrics", Some(HAMMOND)),
                        person("Thom Yorke", "Composer", "Composition & Lyrics", None),
                        person("Thom Yorke", "Lyricist", "Composition & Lyrics", None),
                        person("Colin Greenwood", "Bass Guitar", "Performers", None),
                        person("Thom Yorke", "Voice", "Performers", None),
                        person("Sean Slade", "Producer", "Production & Engineering", None),
                        person("Paul Q. Kolderie", "Mixing Engineer", "Production & Engineering", None),
                        person("Radiohead", "Producer", "Production & Engineering", Some(RADIOHEAD)),
                    ]},
                    "sources": {"items": [{"name": "XL Recordings"}]},
                },
            }
        })
    }

    #[test]
    fn asks_for_the_first_hundred_contributors() {
        assert_eq!(
            credits_variables("spotify:track:70LcF31zb1H0PyJoS1Sx1r"),
            json!({
                "trackUri": "spotify:track:70LcF31zb1H0PyJoS1Sx1r",
                "contributorsLimit": 100,
                "contributorsOffset": 0,
            })
        );
    }

    #[test]
    fn groups_credits_like_spotify() {
        let credits = parse_credits("spotify:track:70LcF31zb1H0PyJoS1Sx1r", &creep()).unwrap();
        assert_eq!(credits.track, "Creep");
        assert_eq!(credits.sources, ["XL Recordings"]);
        let names: Vec<&str> = credits
            .groups
            .iter()
            .map(|group| group.name.as_str())
            .collect();
        assert_eq!(
            names,
            [
                "Artist",
                "Composition & Lyrics",
                "Performers",
                "Production & Engineering"
            ]
        );
        assert_eq!(
            credits.groups[0].people,
            [Contributor {
                name: "Radiohead".into(),
                roles: vec!["Main Artist".into()],
                uri: Some(RADIOHEAD.into()),
                url: None,
            }]
        );
        let writers = &credits.groups[1].people;
        assert_eq!(writers.len(), 2);
        assert_eq!(writers[0].uri, None);
        assert_eq!(writers[1].name, "Thom Yorke");
        assert_eq!(writers[1].roles, ["Composer", "Lyricist"]);
        let producers = &credits.groups[3].people;
        assert_eq!(producers[2].name, "Radiohead");
        assert_eq!(producers[2].uri, None);
        assert_eq!(credits.linked_artists(), [RADIOHEAD]);
    }

    #[test]
    fn keeps_external_urls_and_an_other_group() {
        let contributors = [
            json!({"name": "Studio Band", "role": "Performer", "url": "https://example.com/band"}),
            json!({"name": "Studio Band", "role": "Arranger", "reference": {"url": "https://example.com/other"}}),
            json!({"name": "", "role": "Ghost"}),
            json!({"name": "Someone", "role": ""}),
        ];
        let groups = group(&contributors, &HashSet::new());
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].name, OTHER_GROUP);
        assert_eq!(
            groups[0].people,
            [
                Contributor {
                    name: "Studio Band".into(),
                    roles: vec!["Performer".into(), "Arranger".into()],
                    uri: None,
                    url: Some("https://example.com/band".into()),
                },
                Contributor {
                    name: "Someone".into(),
                    roles: Vec::new(),
                    uri: None,
                    url: None,
                },
            ]
        );
    }

    #[test]
    fn refuses_missing_tracks() {
        let error = parse_credits(
            "spotify:track:70LcF31zb1H0PyJoS1Sx1r",
            &json!({"trackUnion": {"__typename": "NotFound"}}),
        )
        .unwrap_err();
        assert_eq!(error.downcast_ref::<Unavailable>(), Some(&Unavailable));
        assert_eq!(error.to_string(), "Credits aren’t available for this song.");
        let empty = parse_credits(
            "spotify:track:70LcF31zb1H0PyJoS1Sx1r",
            &json!({"trackUnion": {"__typename": "Track", "name": "Quiet"}}),
        )
        .unwrap();
        assert!(empty.groups.is_empty());
        assert!(empty.sources.is_empty());
    }

    #[test]
    fn picks_the_cover_color_query_per_kind() {
        let names: Vec<Option<&str>> = [
            "spotify:track:70LcF31zb1H0PyJoS1Sx1r",
            "spotify:album:1b5KESuOMQomF3wntoLAsk",
            "spotify:artist:4Z8W4fKeB5YxbusRsdQVPb",
            "spotify:playlist:37i9dQZF1DXcBWIGoYBM5M",
            "spotify:episode:0123456789abcdefghijkl",
            "spotify:show:0123456789abcdefghijkl",
            "spotify:user:someone",
        ]
        .into_iter()
        .map(|uri| cover_color_operation(uri).map(|operation| operation.name))
        .collect();
        assert_eq!(
            names,
            [
                Some("fetchExtractedColorForTrackEntity"),
                Some("fetchExtractedColorForAlbumEntity"),
                Some("fetchExtractedColorForArtistEntity"),
                Some("fetchExtractedColorForPlaylistEntity"),
                Some("fetchExtractedColorForEpisodeEntity"),
                Some("fetchExtractedColorForPodcastEntity"),
                None,
            ]
        );
    }

    #[test]
    fn reads_the_dark_cover_color() {
        let color = |data: Value| parse_cover_color(&data);
        let swatch = json!({"extractedColors": {"colorDark": {"hex": "#4F3B2A"}}});
        assert_eq!(
            color(json!({"trackUnion": {"albumOfTrack": {"coverArt": swatch.clone()}}})),
            Some(0x4f3b2a)
        );
        assert_eq!(
            color(json!({"playlistV2": {"images": {"items": [swatch.clone()]}}})),
            Some(0x4f3b2a)
        );
        assert_eq!(
            color(json!({"artistUnion": {"visuals": {"avatarImage": swatch}}})),
            Some(0x4f3b2a)
        );
        assert_eq!(color(json!({"albumUnion": {"coverArt": {}}})), None);
        assert_eq!(color(json!({})), None);
    }
}
