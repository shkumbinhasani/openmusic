use std::collections::HashSet;

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    api::Api,
    entity::{self, Card, Kind},
    home::{Layout, Section, count, label, next_offset, section_cards},
    pathfinder::{self, Operation},
};

pub const BROWSE_ALL: Operation = Operation::query(
    "browseAll",
    "dbd8b55e09a58afc52eab438bc228ba28fd72ac2f2148c6c26354980e4579001",
);
pub const BROWSE_PAGE: Operation = Operation::query(
    "browsePage",
    "f5c4e6d668f5716464a231c1cc8b22c1cbf6ad68b09929fd7de813a30581298b",
);
pub const BROWSE_SECTION: Operation = Operation::query(
    "browseSection",
    "b13c1cccbfcb6947753c2613411b3566485c21fd5f36d80a80bb64be61ba2d51",
);

pub const INTEGRATION: &str = "INTEGRATION_DESKTOP";
pub const ALL_SECTIONS: u32 = 10;
pub const ALL_ITEMS: u32 = 99;
pub const PAGE_SECTIONS: u32 = 10;
pub const PAGE_ITEMS: u32 = 10;
pub const SECTION_PAGE: u32 = 20;
pub const HEADER_IMAGE: u32 = 640;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct BrowseAll {
    pub title: String,
    pub tiles: Vec<Card>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BrowsePage {
    pub uri: String,
    pub title: String,
    pub subtitle: Option<String>,
    pub color: Option<u32>,
    pub image: Option<String>,
    pub sections: Vec<Section>,
    pub total: u32,
    pub next: Option<u32>,
}

impl BrowsePage {
    pub fn images(&self) -> Vec<String> {
        self.image
            .iter()
            .cloned()
            .chain(self.sections.iter().flat_map(Section::images))
            .collect()
    }

    pub fn append(&mut self, more: BrowsePage) -> usize {
        let known: HashSet<String> = self
            .sections
            .iter()
            .map(|section| section.uri.clone())
            .collect();
        let before = self.sections.len();
        self.sections.extend(
            more.sections
                .into_iter()
                .filter(|section| section.uri.is_empty() || !known.contains(&section.uri)),
        );
        let added = self.sections.len() - before;
        self.total = self.total.max(more.total);
        self.next = if added == 0 { None } else { more.next };
        added
    }
}

fn base62_id(id: &str) -> bool {
    id.len() == 22 && id.bytes().all(|byte| byte.is_ascii_alphanumeric())
}

pub fn page_uri(uri_or_id: &str) -> String {
    let text = uri_or_id.trim();
    let id = text
        .strip_prefix("spotify:page:")
        .or_else(|| text.strip_prefix("spotify:genre:"))
        .unwrap_or(text);
    if text.starts_with("spotify:page:") || base62_id(id) {
        format!("spotify:page:{id}")
    } else {
        format!("spotify:genre:{id}")
    }
}

pub fn all_variables() -> Value {
    json!({
        "pagePagination": {"offset": 0, "limit": ALL_SECTIONS},
        "sectionPagination": {"offset": 0, "limit": ALL_ITEMS},
        "browseEndUserIntegration": INTEGRATION,
    })
}

pub fn page_variables(uri: &str, offset: u32) -> Value {
    json!({
        "pagePagination": {"offset": offset, "limit": PAGE_SECTIONS},
        "sectionPagination": {"offset": 0, "limit": PAGE_ITEMS},
        "uri": uri,
        "browseEndUserIntegration": INTEGRATION,
        "includeEpisodeContentRatingsV2": false,
    })
}

pub fn section_variables(uri: &str, offset: u32) -> Value {
    json!({
        "pagination": {"offset": offset, "limit": SECTION_PAGE},
        "uri": uri,
        "browseEndUserIntegration": INTEGRATION,
        "includeEpisodeContentRatingsV2": false,
    })
}

pub fn layout(typename: &str) -> Option<Layout> {
    Some(match typename {
        "BrowseGenericSectionData" | "BrowseSingleItemSectionData" => Layout::Shelf,
        "BrowseGridSectionData" | "BrowseRelatedSectionData" => Layout::Tiles,
        _ => return None,
    })
}

fn tiles_only(cards: Vec<Card>, layout: Layout) -> Vec<Card> {
    match layout {
        Layout::Tiles => cards
            .into_iter()
            .filter(|card| card.kind == Kind::Genre)
            .collect(),
        _ => cards,
    }
}

fn section_as(value: &Value, layout: Layout, offset: u32) -> Section {
    let data = &value["data"];
    let items = &value["sectionItems"];
    let cards = tiles_only(section_cards(items), layout);
    Section {
        uri: value["uri"].as_str().unwrap_or_default().to_owned(),
        title: label(&data["title"]),
        subtitle: label(&data["subtitle"]),
        layout,
        header: None,
        total: count(&items["totalCount"]).max(cards.len() as u32),
        next: next_offset(&items["pagingInfo"], offset),
        cards,
    }
}

pub fn parse_section_entry(value: &Value) -> Option<Section> {
    let layout = layout(value["data"]["__typename"].as_str()?)?;
    let section = section_as(value, layout, 0);
    (!section.cards.is_empty()).then_some(section)
}

pub fn parse_all(data: &Value) -> BrowseAll {
    let mut title = String::new();
    let mut seen = HashSet::new();
    let mut tiles = Vec::new();
    for value in data["browseStart"]["sections"]["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|value| value["data"]["__typename"] == "BrowseGridSectionData")
    {
        let section = section_as(value, Layout::Tiles, 0);
        if title.is_empty() {
            title = section.title;
        }
        tiles.extend(
            section
                .cards
                .into_iter()
                .filter(|card| seen.insert(card.uri.clone())),
        );
    }
    BrowseAll { title, tiles }
}

fn header_image(header: &Value) -> Option<String> {
    let image = &header["backgroundImage"];
    [&image["sources"], &image["data"]["sources"]]
        .into_iter()
        .find_map(|sources| entity::pick_image(sources, HEADER_IMAGE))
}

pub fn parse_page(uri: &str, data: &Value, offset: u32) -> BrowsePage {
    let browse = &data["browse"];
    let header = &browse["header"];
    let sections = &browse["sections"];
    let subtitle = label(&header["subtitle"]);
    BrowsePage {
        uri: browse["uri"].as_str().unwrap_or(uri).to_owned(),
        title: label(&header["title"]),
        subtitle: (!subtitle.is_empty()).then_some(subtitle),
        color: header["color"]["hex"].as_str().and_then(entity::color),
        image: header_image(header),
        sections: sections["items"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(parse_section_entry)
            .collect(),
        total: count(&sections["totalCount"]),
        next: next_offset(&sections["pagingInfo"], offset),
    }
}

pub fn parse_section(uri: &str, data: &Value, offset: u32) -> Result<Section> {
    let value = &data["browseSection"];
    if !value.is_object() {
        bail!("Spotify sent no items for this section.");
    }
    let layout = value["data"]["__typename"]
        .as_str()
        .and_then(layout)
        .unwrap_or(Layout::Shelf);
    let mut section = section_as(value, layout, offset);
    if section.uri.is_empty() {
        section.uri = uri.to_owned();
    }
    Ok(section)
}

pub async fn browse_all(api: &Api) -> Result<BrowseAll> {
    let data = pathfinder::query(api, BROWSE_ALL, all_variables()).await?;
    Ok(parse_all(&data))
}

pub async fn browse_page(api: &Api, uri: &str, offset: u32) -> Result<BrowsePage> {
    let uri = page_uri(uri);
    let data = pathfinder::query(api, BROWSE_PAGE, page_variables(&uri, offset)).await?;
    Ok(parse_page(&uri, &data, offset))
}

pub async fn browse_section(api: &Api, uri: &str, offset: u32) -> Result<Section> {
    let data = pathfinder::query(api, BROWSE_SECTION, section_variables(uri, offset)).await?;
    parse_section(uri, &data, offset)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tile(id: &str, title: &str, hex: &str) -> Value {
        json!({"uri": format!("spotify:page:{id}"), "content": {
            "__typename": "BrowseSectionContainerWrapper", "data": {
                "__typename": "BrowseSectionContainer", "data": {"cardRepresentation": {
                    "title": {"transformedLabel": title},
                    "backgroundColor": {"hex": hex},
                    "artwork": {"sources": [{"url": format!("https://i.scdn.co/image/{id}"), "width": 300, "height": 300}]}
                }}
            }
        }})
    }

    fn xlink() -> Value {
        json!({"uri": "spotify:xlink:0JQ5DAozXW0GUBAKjHsifL", "content": {
            "__typename": "BrowseXlinkResponseWrapper", "data": {
                "__typename": "BrowseClientFeature", "featureUri": "spotify:concerts",
                "title": {"transformedLabel": "Live Events"}, "backgroundColor": {"hex": "#8400e7"}
            }
        }})
    }

    fn playlist(id: &str) -> Value {
        json!({"uri": format!("spotify:playlist:{id}"), "content": {
            "__typename": "PlaylistResponseWrapper", "data": {
                "__typename": "Playlist", "name": format!("List {id}"), "format": "", "description": "Songs",
                "uri": format!("spotify:playlist:{id}"),
                "images": {"items": [{"sources": [{"url": format!("https://i.scdn.co/image/{id}"), "width": 300, "height": 300}]}]}
            }
        }})
    }

    fn missing() -> Value {
        json!({"uri": "spotify:playlist:gone", "content": {
            "__typename": "PlaylistResponseWrapper", "data": {"__typename": "NotFound"}
        }})
    }

    #[test]
    fn maps_genre_links_to_page_uris() {
        assert_eq!(
            page_uri("0JQ5DAqbMKFDXXwE9BDJAr"),
            "spotify:page:0JQ5DAqbMKFDXXwE9BDJAr"
        );
        assert_eq!(
            page_uri("spotify:genre:0JQ5DAqbMKFDXXwE9BDJAr"),
            "spotify:page:0JQ5DAqbMKFDXXwE9BDJAr"
        );
        assert_eq!(
            page_uri("spotify:page:0JQ5DAqbMKFSi39LMRT0Cy"),
            "spotify:page:0JQ5DAqbMKFSi39LMRT0Cy"
        );
        assert_eq!(
            page_uri("spotify:genre:entertainment_v2_podcasts-page"),
            "spotify:genre:entertainment_v2_podcasts-page"
        );
        assert_eq!(page_uri("pop"), "spotify:genre:pop");
    }

    #[test]
    fn builds_the_desktop_browse_variables() {
        assert_eq!(
            all_variables(),
            json!({"pagePagination": {"offset": 0, "limit": 10}, "sectionPagination": {"offset": 0, "limit": 99},
                "browseEndUserIntegration": "INTEGRATION_DESKTOP"})
        );
        assert_eq!(
            page_variables("spotify:page:0JQ5DAqbMKFDXXwE9BDJAr", 10),
            json!({"pagePagination": {"offset": 10, "limit": 10}, "sectionPagination": {"offset": 0, "limit": 10},
                "uri": "spotify:page:0JQ5DAqbMKFDXXwE9BDJAr", "browseEndUserIntegration": "INTEGRATION_DESKTOP",
                "includeEpisodeContentRatingsV2": false})
        );
        assert_eq!(
            section_variables("spotify:section:0JQ5IMCbQBLAlPhkJ41ypw", 20),
            json!({"pagination": {"offset": 20, "limit": 20}, "uri": "spotify:section:0JQ5IMCbQBLAlPhkJ41ypw",
                "browseEndUserIntegration": "INTEGRATION_DESKTOP", "includeEpisodeContentRatingsV2": false})
        );
        assert_eq!(
            pathfinder::body(BROWSE_SECTION, Value::Null)["operationName"],
            "browseSection"
        );
    }

    #[test]
    fn keeps_category_tiles_for_browse_all() {
        let reply = json!({"browseStart": {"__typename": "BrowseSectionContainer", "uri": "spotify:page:0JQ5DArNBzkmxXHCBROWSE",
        "sections": {"items": [
            {"uri": "spotify:section:promo", "data": {"__typename": "BrowseGenericSectionData", "title": {"transformedLabel": "Promo"}},
             "sectionItems": {"items": [playlist("p1")]}},
            {"uri": "spotify:section:0JQ5DAwD41j2mAgQ1AtEZl", "data": {"__typename": "BrowseGridSectionData", "title": {"transformedLabel": "Alles durchsuchen"}},
             "sectionItems": {"items": [
                tile("0JQ5DAqbMKFSi39LMRT0Cy", "Musik", "#dc148c"), xlink(),
                tile("0JQ5DAqbMKFDXXwE9BDJAr", "Rock", "#AF2896"), tile("0JQ5DAqbMKFSi39LMRT0Cy", "Musik", "#dc148c")
             ]}}
        ]}}});
        let all = parse_all(&reply);
        assert_eq!(all.title, "Alles durchsuchen");
        let tiles: Vec<(&str, Option<u32>)> = all
            .tiles
            .iter()
            .map(|card| (card.title.as_str(), card.color))
            .collect();
        assert_eq!(tiles, [("Musik", Some(0xdc148c)), ("Rock", Some(0xaf2896))]);
        assert_eq!(
            all.tiles[0].image.as_deref(),
            Some("https://i.scdn.co/image/0JQ5DAqbMKFSi39LMRT0Cy")
        );
    }

    fn page_reply(next: Value) -> Value {
        json!({"browse": {"__typename": "BrowseSectionContainer", "uri": "spotify:page:0JQ5DAqbMKFDXXwE9BDJAr",
        "header": {"title": {"transformedLabel": "Rock"}, "subtitle": null, "color": {"hex": "#AF2896"}, "backgroundImage": null},
        "sections": {"pagingInfo": {"nextOffset": next}, "totalCount": 11, "items": [
            {"__typename": "BrowseSection", "uri": "spotify:section:pulse", "targetLocation": "",
             "data": {"__typename": "BrowseGenericSectionData", "title": {"transformedLabel": "The pulse of Rock"}, "subtitle": null},
             "sectionItems": {"items": [playlist("a"), missing(), playlist("b")], "totalCount": 19}},
            {"__typename": "BrowseSection", "uri": "spotify:section:grid",
             "data": {"__typename": "BrowseGridSectionData", "title": {"transformedLabel": "Browse all"}},
             "sectionItems": {"items": [tile("0JQ5DAqbMKFDXXwE9BDJAr", "Rock", "#AF2896"), xlink()]}},
            {"__typename": "BrowseSection", "uri": "spotify:section:related",
             "data": {"__typename": "BrowseRelatedSectionData"},
             "sectionItems": {"items": [tile("0JQ5DArNBzkmxXHCqFLx3c", "Explore all categories", "#777777")]}},
            {"__typename": "BrowseSection", "uri": "spotify:section:odd",
             "data": {"__typename": "BrowseUnsupportedSectionData"},
             "sectionItems": {"items": [playlist("c")]}},
            {"__typename": "BrowseSection", "uri": "spotify:section:empty",
             "data": {"__typename": "BrowseGenericSectionData", "title": {"transformedLabel": "Gone"}},
             "sectionItems": {"items": [missing()]}}
        ]}}})
    }

    #[test]
    fn maps_a_genre_page() {
        let page = parse_page(
            "spotify:page:0JQ5DAqbMKFDXXwE9BDJAr",
            &page_reply(json!(10)),
            0,
        );
        assert_eq!(page.title, "Rock");
        assert_eq!(page.subtitle, None);
        assert_eq!(page.color, Some(0xaf2896));
        assert_eq!(page.image, None);
        assert_eq!(page.total, 11);
        assert_eq!(page.next, Some(10));
        let sections: Vec<(Layout, &str, usize, u32)> = page
            .sections
            .iter()
            .map(|section| {
                (
                    section.layout,
                    section.title.as_str(),
                    section.cards.len(),
                    section.total,
                )
            })
            .collect();
        assert_eq!(
            sections,
            [
                (Layout::Shelf, "The pulse of Rock", 2, 19),
                (Layout::Tiles, "Browse all", 1, 1),
                (Layout::Tiles, "", 1, 1),
            ]
        );
        assert!(page.sections[0].has_more(4));
        assert_eq!(page.images().len(), 4);
    }

    #[test]
    fn pages_through_genre_sections() {
        let mut page = parse_page(
            "spotify:page:0JQ5DAqbMKFDXXwE9BDJAr",
            &page_reply(json!(10)),
            0,
        );
        let mut more = parse_page(
            "spotify:page:0JQ5DAqbMKFDXXwE9BDJAr",
            &page_reply(Value::Null),
            10,
        );
        more.sections[0].uri = "spotify:section:later".into();
        assert_eq!(page.append(more), 1);
        assert_eq!(page.sections.len(), 4);
        assert_eq!(page.next, None);
        let again = parse_page(
            "spotify:page:0JQ5DAqbMKFDXXwE9BDJAr",
            &page_reply(json!(20)),
            10,
        );
        assert_eq!(page.append(again), 0);
        assert_eq!(page.next, None);
        let stuck = parse_page("spotify:page:x", &page_reply(json!(10)), 10);
        assert_eq!(stuck.next, None);
    }

    #[test]
    fn maps_a_header_image() {
        let mut reply = page_reply(Value::Null);
        reply["browse"]["header"]["backgroundImage"] = json!({"sources": [
            {"url": "https://i.scdn.co/image/small", "width": 300, "height": 300},
            {"url": "https://i.scdn.co/image/large", "width": 640, "height": 640}
        ]});
        reply["browse"]["header"]["subtitle"] = json!({"transformedLabel": "Podcasts"});
        let page = parse_page("spotify:genre:entertainment_v2_podcasts-page", &reply, 0);
        assert_eq!(page.image.as_deref(), Some("https://i.scdn.co/image/large"));
        assert_eq!(page.subtitle.as_deref(), Some("Podcasts"));
    }

    #[test]
    fn maps_a_browse_show_all_page() {
        let reply = json!({"browseSection": {"__typename": "BrowseSection",
            "data": {"__typename": "BrowseGenericSectionData", "title": {"transformedLabel": "Hand-picked new releases"}, "subtitle": null},
            "sectionItems": {"items": [playlist("a"), playlist("b")], "pagingInfo": {"nextOffset": 40}, "totalCount": 615}}});
        let section = parse_section("spotify:section:0JQ5IMCbQBLAlPhkJ41ypw", &reply, 20).unwrap();
        assert_eq!(section.uri, "spotify:section:0JQ5IMCbQBLAlPhkJ41ypw");
        assert_eq!(section.title, "Hand-picked new releases");
        assert_eq!(section.cards.len(), 2);
        assert_eq!(section.total, 615);
        assert_eq!(section.next, Some(40));
        let mut last = reply.clone();
        last["browseSection"]["sectionItems"]["pagingInfo"]["nextOffset"] = json!(0);
        assert_eq!(
            parse_section("spotify:section:x", &last, 20).unwrap().next,
            None
        );
        assert!(parse_section("spotify:section:x", &json!({"browseSection": null}), 0).is_err());
    }
}
