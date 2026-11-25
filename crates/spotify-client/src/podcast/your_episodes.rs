use std::collections::HashMap;

use anyhow::Result;
use futures::{StreamExt, stream};

use super::{Episode, Played, Unavailable};
use crate::{
    api::Api,
    catalog::{self, ContextItem},
    collection::{self, Item},
};

pub const CONTEXT: &str = "spotify:collection:your-episodes";
pub const RESOLVE_LIMIT: usize = 2000;
pub const BATCH: usize = 20;
const PARALLEL: usize = 4;
const SECONDS_BEFORE: i64 = 100_000_000_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub uri: String,
    pub added_ms: Option<i64>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Sort {
    #[default]
    Added,
    Title,
    Released,
    Show,
}

impl Sort {
    pub const ALL: [Sort; 4] = [Sort::Added, Sort::Title, Sort::Released, Sort::Show];

    pub fn label(self) -> &'static str {
        match self {
            Self::Added => "Date added",
            Self::Title => "Title",
            Self::Released => "Release date",
            Self::Show => "Podcast",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Added => "added",
            Self::Title => "title",
            Self::Released => "released",
            Self::Show => "show",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|sort| sort.name() == name)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Filter {
    Unplayed,
    InProgress,
    Video,
}

impl Filter {
    pub const ALL: [Filter; 3] = [Filter::Unplayed, Filter::InProgress, Filter::Video];

    pub fn label(self) -> &'static str {
        match self {
            Self::Unplayed => "Unplayed",
            Self::InProgress => "In Progress",
            Self::Video => "Video",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Unplayed => "unplayed",
            Self::InProgress => "inprogress",
            Self::Video => "video",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|filter| filter.name() == name)
    }

    pub fn keeps(self, episode: &Episode, played: Played) -> bool {
        match self {
            Self::Unplayed => !played.finished(),
            Self::InProgress => played.started(),
            Self::Video => episode.video,
        }
    }
}

pub fn added_ms(raw: i64) -> Option<i64> {
    match raw {
        ..=0 => None,
        seconds if seconds < SECONDS_BEFORE => Some(seconds * 1000),
        milliseconds => Some(milliseconds),
    }
}

pub fn merge(saved: &[Item], resolved: &[ContextItem]) -> Vec<Entry> {
    let mut entries: Vec<Entry> = Vec::new();
    let mut positions: HashMap<String, usize> = HashMap::new();
    let found = saved
        .iter()
        .map(|item| (&item.uri, added_ms(item.added_at)))
        .chain(
            resolved
                .iter()
                .map(|item| (&item.uri, item.added_at().and_then(added_ms))),
        );
    for (uri, added) in found {
        if !uri.starts_with("spotify:episode:") {
            continue;
        }
        match positions.get(uri) {
            Some(&position) => {
                let entry = &mut entries[position];
                entry.added_ms = entry.added_ms.or(added);
            }
            None => {
                positions.insert(uri.clone(), entries.len());
                entries.push(Entry {
                    uri: uri.clone(),
                    added_ms: added,
                });
            }
        }
    }
    entries.sort_by_key(|entry| std::cmp::Reverse(entry.added_ms));
    entries
}

pub async fn entries(api: &Api, user: &str) -> Result<Vec<Entry>> {
    let (saved, resolved) = tokio::join!(
        collection::all(api, user, collection::LISTEN_LATER),
        catalog::resolve(api, CONTEXT, RESOLVE_LIMIT),
    );
    match (saved, resolved) {
        (Err(error), Err(_)) => Err(error),
        (saved, resolved) => Ok(merge(
            &saved.unwrap_or_default(),
            &resolved.map(|resolved| resolved.items).unwrap_or_default(),
        )),
    }
}

pub fn settle(found: Vec<Result<Episode>>) -> Result<Vec<Option<Episode>>> {
    found
        .into_iter()
        .map(|result| match result {
            Ok(episode) => Ok(Some(episode)),
            Err(error) if error.downcast_ref::<Unavailable>().is_some() => Ok(None),
            Err(error) => Err(error),
        })
        .collect()
}

pub async fn decorate(api: &Api, uris: &[String]) -> Result<Vec<Option<Episode>>> {
    let found: Vec<Result<Episode>> = stream::iter(uris.iter().cloned())
        .map(|uri| async move { super::episode(api, &uri).await })
        .buffered(PARALLEL)
        .collect()
        .await;
    settle(found)
}

pub fn arrange<'a>(
    episodes: &[&'a Episode],
    played: impl Fn(&Episode) -> Played,
    sort: Sort,
    filter: Option<Filter>,
) -> Vec<&'a Episode> {
    let mut kept: Vec<&Episode> = episodes
        .iter()
        .copied()
        .filter(|episode| filter.is_none_or(|filter| filter.keeps(episode, played(episode))))
        .collect();
    match sort {
        Sort::Added => {}
        Sort::Title => kept.sort_by_cached_key(|episode| episode.name.to_lowercase()),
        Sort::Released => kept.sort_by_key(|episode| std::cmp::Reverse(episode.released.clone())),
        Sort::Show => kept.sort_by_cached_key(|episode| episode.show_name.to_lowercase()),
    }
    kept
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn item(uri: &str, added_at: i64) -> Item {
        Item {
            uri: uri.to_owned(),
            context_uri: None,
            added_at,
        }
    }

    fn resolved(uri: &str, added_at: Option<&str>) -> ContextItem {
        ContextItem {
            uri: uri.to_owned(),
            uid: None,
            metadata: added_at
                .map(|added| HashMap::from([("added_at".to_owned(), added.to_owned())]))
                .unwrap_or_default(),
        }
    }

    fn episode(id: &str, name: &str, show: &str, released: &str, video: bool) -> Episode {
        Episode {
            uri: format!("spotify:episode:{id}"),
            name: name.to_owned(),
            show_name: show.to_owned(),
            released: Some(released.to_owned()),
            video,
            ..Episode::default()
        }
    }

    #[test]
    fn reads_seconds_and_milliseconds() {
        assert_eq!(added_ms(0), None);
        assert_eq!(added_ms(-5), None);
        assert_eq!(added_ms(1_725_636_253), Some(1_725_636_253_000));
        assert_eq!(added_ms(1_725_636_253_000), Some(1_725_636_253_000));
    }

    #[test]
    fn merges_both_stores_newest_first() {
        let saved = [
            item("spotify:episode:00000000000000000000a1", 1_700_000_000),
            item("spotify:episode:00000000000000000000b1", 0),
            item("spotify:track:00000000000000000000t1", 1_800_000_000),
        ];
        let context = [
            resolved(
                "spotify:episode:00000000000000000000b1",
                Some("1750000000000"),
            ),
            resolved(
                "spotify:episode:00000000000000000000a1",
                Some("1600000000000"),
            ),
            resolved("spotify:episode:00000000000000000000c1", None),
            resolved(
                "spotify:episode:00000000000000000000d1",
                Some("1760000000000"),
            ),
        ];
        let merged = merge(&saved, &context);
        let uris: Vec<&str> = merged.iter().map(|entry| entry.uri.as_str()).collect();
        assert_eq!(
            uris,
            [
                "spotify:episode:00000000000000000000d1",
                "spotify:episode:00000000000000000000b1",
                "spotify:episode:00000000000000000000a1",
                "spotify:episode:00000000000000000000c1",
            ]
        );
        assert_eq!(merged[1].added_ms, Some(1_750_000_000_000));
        assert_eq!(merged[2].added_ms, Some(1_700_000_000_000));
        assert_eq!(merged[3].added_ms, None);
        assert!(merge(&[], &[]).is_empty());
    }

    #[test]
    fn keeps_unavailable_episodes_out_and_stops_on_errors() {
        let found = vec![
            Ok(episode("a", "A", "S", "2026-01-01", false)),
            Err(anyhow::Error::new(Unavailable::Restricted)),
            Err(anyhow::Error::new(Unavailable::Missing)),
        ];
        let settled = settle(found).unwrap();
        assert_eq!(settled.len(), 3);
        assert!(settled[0].is_some());
        assert!(settled[1].is_none() && settled[2].is_none());
        let failed = vec![
            Ok(episode("a", "A", "S", "2026-01-01", false)),
            Err(anyhow::anyhow!("offline")),
        ];
        assert!(settle(failed).is_err());
    }

    #[test]
    fn sorts_and_filters_locally() {
        let first = episode("a", "beta", "Zulu", "2026-03-01T00:00:00Z", false);
        let second = episode("b", "Alpha", "Mike", "2026-05-01T00:00:00Z", true);
        let third = episode("c", "gamma", "Alpha", "2025-01-01T00:00:00Z", false);
        let all = [&first, &second, &third];
        let played = |episode: &Episode| match episode.uri.as_str() {
            "spotify:episode:a" => Played::Completed,
            "spotify:episode:b" => Played::InProgress(10),
            _ => Played::NotStarted,
        };
        let names = |list: Vec<&Episode>| list.iter().map(|e| e.name.clone()).collect::<Vec<_>>();
        assert_eq!(
            names(arrange(&all, played, Sort::Added, None)),
            ["beta", "Alpha", "gamma"]
        );
        assert_eq!(
            names(arrange(&all, played, Sort::Title, None)),
            ["Alpha", "beta", "gamma"]
        );
        assert_eq!(
            names(arrange(&all, played, Sort::Released, None)),
            ["Alpha", "beta", "gamma"]
        );
        assert_eq!(
            names(arrange(&all, played, Sort::Show, None)),
            ["gamma", "Alpha", "beta"]
        );
        assert_eq!(
            names(arrange(&all, played, Sort::Added, Some(Filter::Unplayed))),
            ["Alpha", "gamma"]
        );
        assert_eq!(
            names(arrange(&all, played, Sort::Added, Some(Filter::InProgress))),
            ["Alpha"]
        );
        assert_eq!(
            names(arrange(&all, played, Sort::Title, Some(Filter::Video))),
            ["Alpha"]
        );
    }

    #[test]
    fn names_round_trip() {
        for sort in Sort::ALL {
            assert_eq!(Sort::from_name(sort.name()), Some(sort));
        }
        for filter in Filter::ALL {
            assert_eq!(Filter::from_name(filter.name()), Some(filter));
        }
        assert_eq!(Sort::from_name("nope"), None);
    }
}
