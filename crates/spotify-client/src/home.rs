use std::collections::HashSet;

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    api::Api,
    entity::{self, Card, Kind},
    pathfinder::{self, Operation, PathfinderError},
};

pub const HOME: Operation = Operation::query(
    "home",
    "5fb7da7a03ee2776c0670856609cb85a189a3cecc36cc160b91cde79063b9cf1",
);
pub const HOME_SECTION: Operation = Operation::query(
    "homeSection",
    "5fb7da7a03ee2776c0670856609cb85a189a3cecc36cc160b91cde79063b9cf1",
);

pub const SECTION_ITEMS: u32 = 10;
pub const SHOW_ALL_ITEMS: u32 = 20;
pub const INTEGRATION: &str = "INTEGRATION_DESKTOP";
pub const FEED_TITLE: &str = "More for you";
const SKIPPED_CHIPS: [&str; 1] = ["wrapped-chip"];
const FULL_SHORTCUTS: usize = 8;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Chip {
    pub id: String,
    pub label: String,
    pub sub: Vec<Chip>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Layout {
    Shortcuts,
    Shelf,
    Feed,
    Tiles,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Section {
    pub uri: String,
    pub title: String,
    pub subtitle: String,
    pub layout: Layout,
    pub header: Option<Card>,
    pub cards: Vec<Card>,
    pub total: u32,
    pub next: Option<u32>,
}

impl Section {
    pub fn has_more(&self, shown: usize) -> bool {
        self.next.is_some() || self.cards.len() > shown || self.total as usize > shown
    }

    pub fn images(&self) -> impl Iterator<Item = String> + '_ {
        self.cards
            .iter()
            .chain(self.header.iter())
            .filter_map(|card| card.image.clone())
    }

    pub fn append(&mut self, more: Section) -> usize {
        let known: HashSet<String> = self.cards.iter().map(|card| card.uri.clone()).collect();
        let before = self.cards.len();
        self.cards.extend(
            more.cards
                .into_iter()
                .filter(|card| !known.contains(&card.uri)),
        );
        let added = self.cards.len() - before;
        self.total = self.total.max(more.total);
        self.next = if added == 0 { None } else { more.next };
        added
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Home {
    pub facet: String,
    pub greeting: String,
    pub chips: Vec<Chip>,
    pub sections: Vec<Section>,
}

impl Home {
    pub fn images(&self) -> Vec<String> {
        self.sections.iter().flat_map(Section::images).collect()
    }

    pub fn chip_parent(&self, facet: &str) -> Option<&Chip> {
        self.chips
            .iter()
            .find(|chip| chip.id == facet || chip.sub.iter().any(|sub| sub.id == facet))
    }

    pub fn section(&self, uri: &str) -> Option<&Section> {
        self.sections.iter().find(|section| section.uri == uri)
    }
}

pub fn zone_from_path(path: &str) -> Option<String> {
    let (_, zone) = path.split_once("zoneinfo/")?;
    let zone = zone.trim_matches('/');
    (!zone.is_empty()).then(|| zone.to_owned())
}

pub fn time_zone() -> String {
    std::fs::read_link("/etc/localtime")
        .ok()
        .and_then(|path| zone_from_path(&path.to_string_lossy()))
        .unwrap_or_else(|| "UTC".to_owned())
}

pub fn home_variables(facet: &str, time_zone: &str) -> Value {
    json!({
        "timeZone": time_zone,
        "sp_t": "",
        "facet": facet,
        "sectionItemsLimit": SECTION_ITEMS,
        "includeHomeChapterVideoCards": false,
        "homeEndUserIntegration": INTEGRATION,
        "includeEpisodeContentRatingsV2": false,
    })
}

pub fn section_variables(uri: &str, time_zone: &str) -> Value {
    json!({
        "includeHomeChapterVideoCards": false,
        "uri": uri,
        "homeEndUserIntegration": INTEGRATION,
        "timeZone": time_zone,
        "sp_t": "",
        "sectionItemsOffset": 0,
        "sectionItemsLimit": SHOW_ALL_ITEMS,
        "includeEpisodeContentRatingsV2": false,
    })
}

pub fn label(value: &Value) -> String {
    value["transformedLabel"]
        .as_str()
        .or_else(|| value.as_str())
        .unwrap_or_default()
        .trim()
        .to_owned()
}

pub fn count(value: &Value) -> u32 {
    value
        .as_u64()
        .map_or(0, |count| count.min(u64::from(u32::MAX)) as u32)
}

pub fn next_offset(paging: &Value, offset: u32) -> Option<u32> {
    let next = u32::try_from(paging["nextOffset"].as_u64()?).ok()?;
    (next > offset).then_some(next)
}

fn chip(value: &Value) -> Option<Chip> {
    let id = value["id"].as_str()?.trim();
    let label = label(&value["label"]);
    if id.is_empty() || label.is_empty() || SKIPPED_CHIPS.contains(&id) {
        return None;
    }
    Some(Chip {
        id: id.to_owned(),
        label,
        sub: value["subChips"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(chip)
            .collect(),
    })
}

pub fn section_cards(items: &Value) -> Vec<Card> {
    let mut seen = HashSet::new();
    items["items"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|item| {
            let content = &item["content"];
            if content["__typename"] == "ListResponseWrapper" {
                content["data"]["items"]["items"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(entity::card)
                    .collect()
            } else {
                entity::card(item).into_iter().collect::<Vec<_>>()
            }
        })
        .filter(|card| card.kind != Kind::Feature && seen.insert(card.uri.clone()))
        .collect()
}

pub fn layout(typename: &str) -> Option<Layout> {
    Some(match typename {
        "HomeShortsSectionData" => Layout::Shortcuts,
        "HomeGenericSectionData" | "HomeGridSectionData" | "HomeRecentlyPlayedSectionData" => {
            Layout::Shelf
        }
        "HomeFeedBaselineSectionData" => Layout::Feed,
        _ => return None,
    })
}

fn section_as(value: &Value, layout: Layout) -> Section {
    let data = &value["data"];
    let items = &value["sectionItems"];
    let cards = section_cards(items);
    Section {
        uri: value["uri"].as_str().unwrap_or_default().to_owned(),
        title: label(&data["title"]),
        subtitle: label(&data["subtitle"]),
        layout,
        header: entity::card(&data["headerEntity"]),
        total: count(&items["totalCount"]).max(cards.len() as u32),
        next: next_offset(&items["pagingInfo"], 0),
        cards,
    }
}

pub fn parse_section(value: &Value) -> Option<Section> {
    let layout = layout(value["data"]["__typename"].as_str()?)?;
    Some(section_as(value, layout))
}

fn merge_feed(sections: &mut Vec<Section>, mut section: Section) {
    let reason = (!section.title.is_empty()).then(|| section.title.clone());
    for card in &mut section.cards {
        card.reason = reason.clone();
    }
    if let Some(last) = sections.last_mut()
        && last.layout == Layout::Feed
    {
        let known: HashSet<String> = last.cards.iter().map(|card| card.uri.clone()).collect();
        last.cards.extend(
            section
                .cards
                .into_iter()
                .filter(|card| !known.contains(&card.uri)),
        );
        last.total = last.cards.len() as u32;
        return;
    }
    section.title = FEED_TITLE.to_owned();
    section.subtitle.clear();
    section.header = None;
    section.next = None;
    section.total = section.cards.len() as u32;
    sections.push(section);
}

pub fn parse_home(facet: &str, data: &Value) -> Result<Home> {
    let home = &data["home"];
    if !home.is_object() {
        bail!(PathfinderError::NoData);
    }
    let greeting = label(&home["greeting"]);
    let chips = home["homeChips"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(chip)
        .collect();
    let mut sections = Vec::new();
    for value in home["sectionContainer"]["sections"]["items"]
        .as_array()
        .into_iter()
        .flatten()
    {
        let Some(mut section) = parse_section(value) else {
            continue;
        };
        if section.layout == Layout::Shortcuts {
            if section.title.is_empty() {
                section.title = greeting.clone();
            }
            if section.cards.len() < FULL_SHORTCUTS && section.cards.len() % 2 == 1 {
                section.cards.pop();
            }
        }
        if section.cards.is_empty() {
            continue;
        }
        if section.layout == Layout::Feed {
            merge_feed(&mut sections, section);
        } else {
            sections.push(section);
        }
    }
    Ok(Home {
        facet: facet.to_owned(),
        greeting,
        chips,
        sections,
    })
}

pub fn parse_home_section(uri: &str, data: &Value) -> Result<Section> {
    let sections: Vec<&Value> = data["homeSections"]["sections"]
        .as_array()
        .into_iter()
        .flatten()
        .collect();
    let Some(value) = sections
        .iter()
        .find(|section| section["uri"] == uri)
        .or(sections.first())
    else {
        bail!("Spotify sent no items for this section.");
    };
    let layout = value["data"]["__typename"]
        .as_str()
        .and_then(layout)
        .unwrap_or(Layout::Shelf);
    let mut section = section_as(value, layout);
    if section.uri.is_empty() {
        section.uri = uri.to_owned();
    }
    section.next = None;
    Ok(section)
}

pub async fn home(api: &Api, facet: &str, time_zone: &str) -> Result<Home> {
    let data = pathfinder::query(api, HOME, home_variables(facet, time_zone)).await?;
    parse_home(facet, &data)
}

pub async fn home_section(api: &Api, uri: &str, time_zone: &str) -> Result<Section> {
    let data = pathfinder::query(api, HOME_SECTION, section_variables(uri, time_zone)).await?;
    parse_home_section(uri, &data)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cover(name: &str) -> Value {
        json!({"sources": [{"url": format!("https://i.scdn.co/image/{name}"), "width": 300, "height": 300}]})
    }

    fn album(id: &str) -> Value {
        json!({"uri": format!("spotify:album:{id}"), "data": null, "content": {
            "__typename": "AlbumResponseWrapper", "data": {
                "__typename": "Album", "name": format!("Album {id}"), "albumType": "ALBUM",
                "artists": {"items": [{"profile": {"name": "Band"}, "uri": "spotify:artist:a"}]},
                "coverArt": cover(id)
            }
        }})
    }

    fn playlist(id: &str, name: &str) -> Value {
        json!({"uri": format!("spotify:playlist:{id}"), "data": null, "content": {
            "__typename": "PlaylistResponseWrapper", "data": {
                "__typename": "Playlist", "name": name, "format": "daily-mix", "description": "",
                "uri": format!("spotify:playlist:{id}"),
                "images": {"items": [{"sources": [{"url": format!("https://pickasso.spotifycdn.com/{id}"), "width": null, "height": null}]}]},
                "ownerV2": {"data": {"__typename": "User", "name": "Spotify", "uri": "spotify:user:spotify"}}
            }
        }})
    }

    fn missing(id: &str) -> Value {
        json!({"uri": format!("spotify:playlist:{id}"), "content": {
            "__typename": "PlaylistResponseWrapper", "data": {"__typename": "NotFound"}
        }})
    }

    fn generic(uri: &str, title: &str, items: Vec<Value>, total: u32, next: Value) -> Value {
        json!({"uri": uri, "data": {
            "__typename": "HomeGenericSectionData",
            "title": {"transformedLabel": title, "translatedBaseText": title},
            "subtitle": {"transformedLabel": ""},
            "headerEntity": {"__typename": "UnknownType"}
        }, "sectionItems": {"items": items, "pagingInfo": {"nextOffset": next}, "totalCount": total}})
    }

    fn feed(uri: &str, title: &str, items: Vec<Value>) -> Value {
        json!({"uri": uri, "data": {
            "__typename": "HomeFeedBaselineSectionData",
            "title": {"transformedLabel": title}, "iconName": "",
            "headerEntity": {"__typename": "UnknownType"}
        }, "sectionItems": {"items": items, "totalCount": 1}})
    }

    fn shortcuts() -> Value {
        let liked = json!({"uri": "spotify:user:someone:collection", "content": {
            "__typename": "UnknownTypeWrapper", "_uri": "spotify:user:someone:collection"
        }});
        json!({"uri": "spotify:section:shortcuts", "data": {"__typename": "HomeShortsSectionData"},
        "sectionItems": {
            "items": [album("a1"), album("a2"), album("a3"), liked, album("a5")],
            "pagingInfo": {"nextOffset": 10}, "totalCount": 21
        }})
    }

    fn recent(uid: &str, uri: &str, kind: &str, name: &str) -> Value {
        json!({"uid": uid, "entity": {"__typename": "EntityResponseWrapper", "_uri": uri, "data": {
            "__typename": "Entity", "entityTypeTrait": {"type": kind},
            "identityTrait": {"name": name, "contributors": {"items": [{"name": "Band"}]}}
        }}})
    }

    fn recents() -> Value {
        let entries = vec![
            recent("u1", "spotify:album:r1", "ENTITY_TYPE_ALBUM", "Recent One"),
            recent(
                "u2",
                "spotify:artist:r2",
                "ENTITY_TYPE_ARTIST",
                "Recent Artist",
            ),
        ];
        let list = json!({"uri": "spotify:list:recents:main", "content": {
            "__typename": "ListResponseWrapper",
            "data": {"__typename": "List", "uri": "spotify:list:recents:main",
                "items": {"totalCount": 2, "items": entries}}
        }});
        json!({"uri": "spotify:section:recents",
            "data": {"__typename": "HomeRecentlyPlayedSectionData", "title": {"transformedLabel": "Recents"}},
            "sectionItems": {"items": [list], "totalCount": 1}})
    }

    fn more_like() -> Value {
        let header = json!({"__typename": "ArtistResponseWrapper", "data": {"__typename": "Artist",
            "profile": {"name": "Band"}, "uri": "spotify:artist:a", "visuals": {"avatarImage": cover("band")}}});
        json!({"uri": "spotify:section:more-like", "data": {
                "__typename": "HomeGenericSectionData",
                "title": {"transformedLabel": "More like Band"}, "headerEntity": header
            },
            "sectionItems": {"items": [album("m1")], "pagingInfo": {"nextOffset": 10}, "totalCount": 20}})
    }

    fn chips() -> Value {
        json!([
            {"id": "music-chip", "label": {"transformedLabel": "Music"},
             "subChips": [{"id": "music-following-chip", "label": {"transformedLabel": "Following"}}]},
            {"id": "podcasts-chip", "label": {"transformedLabel": "Podcasts"}, "subChips": []},
            {"id": "wrapped-chip", "label": {"transformedLabel": "Wrapped"}, "subChips": []},
            {"id": "empty-chip", "label": {"transformedLabel": ""}, "subChips": []}
        ])
    }

    fn home_reply() -> Value {
        let made_for = vec![
            playlist("37i9dQZF1E8Redacted001", "Daily Mix 1"),
            missing("37i9dQZF1E8Redacted003"),
            playlist("37i9dQZF1E8Redacted004", "Daily Mix 2"),
        ];
        let dj = json!({"uri": "spotify:section:dj", "data": {"__typename": "HomeYourDJSectionData"},
            "sectionItems": {"items": [album("dj")], "totalCount": 1}});
        let sections = vec![
            shortcuts(),
            generic(
                "spotify:section:made-for",
                "Made For Someone",
                made_for,
                9,
                Value::Null,
            ),
            recents(),
            dj,
            more_like(),
            generic(
                "spotify:section:empty",
                "Empty",
                vec![missing("x")],
                1,
                Value::Null,
            ),
            feed("spotify:section:f1", "Made for you", vec![album("f1")]),
            feed(
                "spotify:section:f2",
                "For fans of Band",
                vec![album("f2"), album("f1")],
            ),
            generic(
                "spotify:section:last",
                "Top picks",
                vec![album("t1")],
                1,
                Value::Null,
            ),
            feed("spotify:section:f3", "", vec![album("f3")]),
        ];
        json!({"home": {
            "__typename": "HomeResponsePayload",
            "greeting": {"transformedLabel": "Good afternoon", "translatedBaseText": "Good afternoon"},
            "homeChips": chips(),
            "sectionContainer": {"uri": "spotify:page:0JQ5DAozXW0GUBAKjHsifH",
                "sections": {"totalCount": 10, "items": sections}}
        }})
    }

    #[test]
    fn builds_the_desktop_home_variables() {
        assert_eq!(
            home_variables("podcasts-chip", "Europe/Belgrade"),
            json!({"timeZone": "Europe/Belgrade", "sp_t": "", "facet": "podcasts-chip",
                "sectionItemsLimit": 10, "includeHomeChapterVideoCards": false,
                "homeEndUserIntegration": "INTEGRATION_DESKTOP", "includeEpisodeContentRatingsV2": false})
        );
        let section = section_variables("spotify:section:x", "UTC");
        assert_eq!(section["uri"], "spotify:section:x");
        assert_eq!(section["sectionItemsLimit"], 20);
        assert_eq!(section["sectionItemsOffset"], 0);
        let body = pathfinder::body(HOME_SECTION, section);
        assert_eq!(body["operationName"], "homeSection");
        assert_eq!(
            body["extensions"]["persistedQuery"]["sha256Hash"],
            HOME.hash
        );
    }

    #[test]
    fn reads_the_time_zone_from_the_localtime_link() {
        assert_eq!(
            zone_from_path("/var/db/timezone/zoneinfo/Europe/Belgrade").as_deref(),
            Some("Europe/Belgrade")
        );
        assert_eq!(
            zone_from_path("/usr/share/zoneinfo/UTC").as_deref(),
            Some("UTC")
        );
        assert_eq!(zone_from_path("/etc/localtime"), None);
        assert_eq!(zone_from_path("/usr/share/zoneinfo/"), None);
        assert!(!time_zone().is_empty());
    }

    #[test]
    fn maps_greeting_and_chips() {
        let home = parse_home("", &home_reply()).unwrap();
        assert_eq!(home.greeting, "Good afternoon");
        let chips: Vec<(&str, &str, usize)> = home
            .chips
            .iter()
            .map(|chip| (chip.id.as_str(), chip.label.as_str(), chip.sub.len()))
            .collect();
        assert_eq!(
            chips,
            [("music-chip", "Music", 1), ("podcasts-chip", "Podcasts", 0)]
        );
        assert_eq!(home.chips[0].sub[0].label, "Following");
        assert_eq!(
            home.chip_parent("music-following-chip")
                .map(|chip| chip.id.as_str()),
            Some("music-chip")
        );
        assert_eq!(home.chip_parent("unknown"), None);
    }

    #[test]
    fn maps_home_sections_in_server_order() {
        let home = parse_home("", &home_reply()).unwrap();
        let sections: Vec<(Layout, &str, usize)> = home
            .sections
            .iter()
            .map(|section| (section.layout, section.title.as_str(), section.cards.len()))
            .collect();
        assert_eq!(
            sections,
            [
                (Layout::Shortcuts, "Good afternoon", 4),
                (Layout::Shelf, "Made For Someone", 2),
                (Layout::Shelf, "Recents", 2),
                (Layout::Shelf, "More like Band", 1),
                (Layout::Feed, FEED_TITLE, 2),
                (Layout::Shelf, "Top picks", 1),
                (Layout::Feed, FEED_TITLE, 1),
            ]
        );
        let shortcuts = &home.sections[0];
        assert_eq!(shortcuts.cards[3].kind, Kind::LikedSongs);
        assert_eq!(shortcuts.next, Some(10));
        assert_eq!(shortcuts.total, 21);
        let made_for = &home.sections[1];
        assert_eq!(made_for.total, 9);
        assert_eq!(made_for.next, None);
        assert!(made_for.has_more(4));
        assert_eq!(made_for.header, None);
        let recents = &home.sections[2];
        assert_eq!(recents.cards[0].uri, "spotify:album:r1");
        assert_eq!(recents.cards[1].kind, Kind::Artist);
        let more_like = &home.sections[3];
        assert_eq!(
            more_like.header.as_ref().map(|card| card.title.as_str()),
            Some("Band")
        );
        assert!(
            home.images()
                .contains(&"https://i.scdn.co/image/band".to_owned())
        );
    }

    #[test]
    fn merges_feed_sections_with_reasons() {
        let home = parse_home("", &home_reply()).unwrap();
        let feed = &home.sections[4];
        let cards: Vec<(&str, Option<&str>)> = feed
            .cards
            .iter()
            .map(|card| (card.uri.as_str(), card.reason.as_deref()))
            .collect();
        assert_eq!(
            cards,
            [
                ("spotify:album:f1", Some("Made for you")),
                ("spotify:album:f2", Some("For fans of Band")),
            ]
        );
        assert_eq!(feed.uri, "spotify:section:f1");
        assert_eq!(feed.total, 2);
        assert!(!feed.has_more(2));
        assert_eq!(home.sections[6].cards[0].reason, None);
    }

    #[test]
    fn keeps_even_shortcut_grids() {
        let mut reply = home_reply();
        let items =
            reply["home"]["sectionContainer"]["sections"]["items"][0]["sectionItems"]["items"]
                .as_array_mut()
                .unwrap();
        items.truncate(3);
        let home = parse_home("", &reply).unwrap();
        assert_eq!(home.sections[0].cards.len(), 2);
        let mut reply = home_reply();
        let items =
            reply["home"]["sectionContainer"]["sections"]["items"][0]["sectionItems"]["items"]
                .as_array_mut()
                .unwrap();
        for index in 0..4 {
            items.push(album(&format!("extra{index}")));
        }
        let home = parse_home("", &reply).unwrap();
        assert_eq!(home.sections[0].cards.len(), 9);
    }

    #[test]
    fn refuses_a_reply_without_a_home() {
        for reply in [json!({"home": null}), json!({})] {
            let error = parse_home("", &reply).unwrap_err();
            assert_eq!(
                error.downcast_ref::<PathfinderError>(),
                Some(&PathfinderError::NoData)
            );
        }
    }

    #[test]
    fn maps_a_home_show_all_reply() {
        let reply = json!({"homeSections": {"__typename": "HomeSectionCollection", "sections": [
            {"__typename": "HomeSection", "uri": "spotify:section:stations",
             "data": {"__typename": "HomeGenericSectionData", "title": {"transformedLabel": "Recommended Stations"}},
             "sectionItems": {"items": [playlist("37i9dQZF1E4Redacted002", "Band Radio"), album("s2")],
                "pagingInfo": {"nextOffset": 20}, "totalCount": 91}}
        ]}});
        let section = parse_home_section("spotify:section:stations", &reply).unwrap();
        assert_eq!(section.title, "Recommended Stations");
        assert_eq!(section.layout, Layout::Shelf);
        assert_eq!(section.cards.len(), 2);
        assert_eq!(section.total, 91);
        assert_eq!(section.next, None);
        assert!(
            parse_home_section(
                "spotify:section:x",
                &json!({"homeSections": {"sections": []}})
            )
            .is_err()
        );
    }

    #[test]
    fn appends_pages_without_duplicates() {
        let reply = home_reply();
        let mut section =
            parse_section(&reply["home"]["sectionContainer"]["sections"]["items"][1]).unwrap();
        let mut more = section.clone();
        more.next = Some(20);
        assert_eq!(section.append(more), 0);
        assert_eq!(section.next, None);
        let mut fresh = section.clone();
        fresh.cards[0].uri = "spotify:playlist:new".into();
        fresh.next = Some(40);
        fresh.total = 30;
        assert_eq!(section.append(fresh), 1);
        assert_eq!(section.next, Some(40));
        assert_eq!(section.total, 30);
    }
}
