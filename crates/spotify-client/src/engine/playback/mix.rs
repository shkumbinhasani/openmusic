use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow};
use serde_json::Value;

use super::{
    Playing,
    join::{Target, track_at},
};
use crate::{
    api::Api,
    automix::{self, ITEM_SPEED, Plan, RECIPE, Side},
    engine::Engine,
    player::Blend,
    playlist::{self, PlaylistItem},
};

const PAGES: usize = 10;
const PAGE: usize = 100;
const PAGE_WAIT: Duration = Duration::from_secs(5);
const FRESH: Duration = Duration::from_secs(15 * 60);
const RETRY: Duration = Duration::from_secs(2 * 60);
const DECORATE: &str = "revision,length,attributes";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Status {
    Loading,
    Ready,
    Failed,
}

impl Status {
    fn name(self) -> &'static str {
        match self {
            Self::Loading => "loading",
            Self::Ready => "ready",
            Self::Failed => "failed",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Entry {
    uid: Option<String>,
    uri: String,
    recipe: String,
}

#[derive(Debug, Default, PartialEq)]
pub(super) struct Read {
    entries: Vec<Entry>,
    fast: HashSet<String>,
    mixed: bool,
    items: usize,
}

impl Read {
    pub(super) fn absorb(&mut self, items: &[PlaylistItem], list: &HashMap<String, String>) {
        self.mixed |= automix::mixed_list(list)
            || items
                .iter()
                .any(|item| automix::mixed_item(&item.attributes));
        self.items += items.len();
        for item in items {
            let key = item.uid.clone().unwrap_or_else(|| item.uri.clone());
            if !automix::plain_speed(automix::item_speed(
                item.attributes.get(ITEM_SPEED).map(String::as_str),
            )) {
                self.fast.insert(key);
            }
            if let Some(recipe) = item.attributes.get(RECIPE).filter(|text| !text.is_empty()) {
                self.entries.push(Entry {
                    uid: item.uid.clone(),
                    uri: item.uri.clone(),
                    recipe: recipe.clone(),
                });
            }
        }
    }
}

#[derive(Debug, Default)]
pub(super) struct Book {
    context: String,
    read: Read,
    status: Option<Status>,
    due: Option<Instant>,
    version: u64,
}

impl Book {
    fn wants_read(&self, context: &str, now: Instant) -> bool {
        self.context != context
            || (self.status != Some(Status::Loading) && self.due.is_none_or(|due| now >= due))
    }

    fn start(&mut self, context: &str) -> u64 {
        if self.context != context {
            *self = Book {
                context: context.to_owned(),
                version: self.version + 1,
                ..Book::default()
            };
        }
        self.status = Some(Status::Loading);
        self.version
    }

    fn finish(&mut self, result: Result<Read>, now: Instant) {
        match result {
            Ok(read) => {
                self.read = read;
                self.status = Some(Status::Ready);
                self.due = Some(now + FRESH);
                self.version += 1;
            }
            Err(_) => {
                self.status = Some(Status::Failed);
                self.due = Some(now + RETRY);
            }
        }
    }

    pub(super) fn candidates(&self, uid: Option<&str>, uris: &[&str]) -> Vec<&str> {
        let entries = &self.read.entries;
        if let Some(uid) = uid
            && entries
                .iter()
                .any(|entry| entry.uid.as_deref() == Some(uid))
        {
            return entries
                .iter()
                .filter(|entry| entry.uid.as_deref() == Some(uid))
                .map(|entry| entry.recipe.as_str())
                .collect();
        }
        entries
            .iter()
            .filter(|entry| uris.contains(&entry.uri.as_str()))
            .map(|entry| entry.recipe.as_str())
            .collect()
    }

    pub(super) fn fast(&self, uid: Option<&str>, uris: &[&str]) -> bool {
        uid.is_some_and(|uid| self.read.fast.contains(uid))
            || uris.iter().any(|uri| self.read.fast.contains(*uri))
    }

    fn describe(&self) -> String {
        format!(
            "automix context={} status={} mixed={} items={} recipes={}",
            self.context,
            self.status.map_or("none", Status::name),
            self.read.mixed,
            self.read.items,
            self.read.entries.len()
        )
    }
}

#[derive(Default)]
pub(super) struct Mix {
    book: Arc<Mutex<Book>>,
    task: Option<tokio::task::JoinHandle<()>>,
    seen: u64,
}

impl Drop for Mix {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

fn page_path(id: &str, from: usize) -> String {
    format!("/playlist/v2/playlist/{id}?decorate={DECORATE}&from={from}&length={PAGE}")
}

async fn read_book(api: &Api, context: &str) -> Result<Read> {
    let id = playlist::playlist_id(context).context("Automix reads playlists only.")?;
    let mut read = Read::default();
    let mut from = 0;
    for _ in 0..PAGES {
        let value: Value = tokio::time::timeout(PAGE_WAIT, api.get_json(&page_path(id, from)))
            .await
            .map_err(|_| anyhow!("The playlist took too long to load."))??;
        let list = playlist::format_attributes(&value["attributes"]["formatAttributes"]);
        let contents = playlist::parse_contents(&value);
        read.absorb(&contents.items, &list);
        match contents.next.filter(|next| *next > from) {
            Some(next) if read.mixed => from = next,
            _ => break,
        }
    }
    Ok(read)
}

pub(super) fn uris_of(track: &Value) -> Vec<&str> {
    let metadata = &track["metadata"];
    [&metadata["uri"], &metadata["linked_from_uri"]]
        .into_iter()
        .filter_map(Value::as_str)
        .filter(|uri| !uri.is_empty())
        .collect()
}

fn speed_of(track: &Value) -> f64 {
    automix::item_speed(track["metadata"][ITEM_SPEED].as_str())
}

pub(super) fn first_plan(
    texts: &[String],
    a: Side,
    b: Side,
    durations: (u32, u32),
) -> Option<Plan> {
    texts
        .iter()
        .filter_map(|text| automix::decode(text))
        .filter(|recipe| recipe.fits(a, b) && recipe.plain_speeds())
        .find_map(|recipe| recipe.plan(durations.0, durations.1))
}

pub(super) fn blend_at(plan: &Plan, position_ms: u32) -> Option<Blend> {
    let late = position_ms.checked_sub(plan.at_ms)?;
    (late.saturating_mul(2) <= plan.length_ms).then(|| Blend {
        length_ms: plan.length_ms,
        lead_in_ms: plan.lead_in_ms,
        elapsed_ms: late,
        shape: plan.shape.clone(),
    })
}

impl Engine {
    fn mix_context(&self) -> Option<String> {
        if !self.audio.settings.automix {
            return None;
        }
        let context = self.playing.as_ref()?.context_uri()?;
        playlist::canonical(&context)
    }

    pub(super) fn follow_mix(&mut self) {
        let Some(context) = self.mix_context() else {
            return;
        };
        let (wanted, version) = match self.audio.mix.book.lock() {
            Ok(book) => (book.wants_read(&context, Instant::now()), book.version),
            Err(_) => return,
        };
        if wanted {
            self.read_mix(context);
        } else if version != self.audio.mix.seen {
            self.audio.mix.seen = version;
            self.check_prepared();
        }
    }

    fn read_mix(&mut self, context: String) {
        if let Some(task) = self.audio.mix.task.take() {
            task.abort();
        }
        let Ok(version) = self
            .audio
            .mix
            .book
            .lock()
            .map(|mut book| book.start(&context))
        else {
            return;
        };
        let api = self.api.clone();
        let book = self.audio.mix.book.clone();
        self.audio.mix.task = Some(tokio::spawn(async move {
            let result = read_book(&api, &context).await;
            if let Ok(mut book) = book.lock()
                && book.context == context
                && book.version == version
            {
                book.finish(result, Instant::now());
            }
        }));
    }

    fn cluster_recipe(&self, uris: &[&str]) -> Option<String> {
        let here = self.playback_here()?;
        let track = here.track.as_ref()?;
        if !uris.contains(&track.uri.as_str()) {
            return None;
        }
        here.track_metadata.get(RECIPE).cloned()
    }

    pub(super) fn mix_plan(
        &self,
        playing: &Playing,
        target: &Target,
        next_ms: u32,
    ) -> Option<Plan> {
        let context = self.mix_context()?;
        let machine = &playing.machine;
        let a = track_at(machine, playing.index)?;
        let b = track_at(machine, target.index)?;
        let a_uris = uris_of(a);
        let b_uris = uris_of(b);
        let music = |uris: &[&str]| {
            uris.first()
                .is_some_and(|uri| uri.starts_with("spotify:track:"))
        };
        if !music(&a_uris) || !music(&b_uris) {
            return None;
        }
        if !automix::plain_speed(speed_of(a)) || !automix::plain_speed(speed_of(b)) {
            return None;
        }
        let a_uid = playing.uid();
        let b_uid = machine["states"]
            .get(target.index)
            .and_then(|state| state["track_uid"].as_str())
            .filter(|uid| !uid.is_empty());
        let mut texts: Vec<String> = a["metadata"][RECIPE]
            .as_str()
            .map(str::to_owned)
            .into_iter()
            .chain(self.cluster_recipe(&a_uris))
            .collect();
        if let Ok(book) = self.audio.mix.book.lock()
            && book.context == context
        {
            if book.fast(a_uid, &a_uris) || book.fast(b_uid, &b_uris) {
                return None;
            }
            texts.extend(
                book.candidates(a_uid, &a_uris)
                    .into_iter()
                    .map(str::to_owned),
            );
        }
        first_plan(
            &texts,
            Side {
                uris: &a_uris,
                uid: a_uid,
            },
            Side {
                uris: &b_uris,
                uid: b_uid,
            },
            (playing.duration_ms, next_ms),
        )
    }

    pub(super) fn mix_report(&self) -> Option<String> {
        if !self.audio.settings.automix {
            return None;
        }
        let book = self.audio.mix.book.lock().ok()?;
        book.status.map(|_| book.describe())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::automix::{MIXED_PLAYLIST, REPORTING, Shape};

    fn item(uid: &str, uri: &str, attributes: &[(&str, &str)]) -> PlaylistItem {
        PlaylistItem {
            uri: uri.to_owned(),
            uid: (!uid.is_empty()).then(|| uid.to_owned()),
            attributes: attributes
                .iter()
                .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
                .collect(),
            ..PlaylistItem::default()
        }
    }

    #[test]
    fn collects_recipes_and_speed_changes_from_a_page() {
        let mut read = Read::default();
        read.absorb(
            &[
                item("00000000000000a1", "spotify:track:a", &[(RECIPE, "AAAA")]),
                item(
                    "00000000000000b2",
                    "spotify:track:b",
                    &[(ITEM_SPEED, "1.1")],
                ),
                item("", "spotify:track:c", &[(RECIPE, "")]),
                item("", "spotify:track:d", &[(ITEM_SPEED, "1")]),
            ],
            &HashMap::new(),
        );
        assert!(read.mixed);
        assert_eq!(read.items, 4);
        assert_eq!(
            read.entries,
            vec![Entry {
                uid: Some("00000000000000a1".into()),
                uri: "spotify:track:a".into(),
                recipe: "AAAA".into(),
            }]
        );
        assert_eq!(read.fast, HashSet::from(["00000000000000b2".to_owned()]));

        let mut plain = Read::default();
        plain.absorb(
            &[item(
                "00000000000000a1",
                "spotify:track:a",
                &[("decision_id", "x")],
            )],
            &HashMap::new(),
        );
        assert!(!plain.mixed);
        let mut marked = Read::default();
        marked.absorb(
            &[item("", "spotify:track:a", &[(REPORTING, MIXED_PLAYLIST)])],
            &HashMap::new(),
        );
        assert!(marked.mixed);
        let mut listed = Read::default();
        listed.absorb(&[], &HashMap::from([("mix".to_owned(), "true".to_owned())]));
        assert!(listed.mixed);
    }

    #[test]
    fn finds_the_recipe_by_row_then_by_song() {
        let mut book = Book::default();
        book.read.absorb(
            &[
                item("00000000000000a1", "spotify:track:a", &[(RECIPE, "first")]),
                item("00000000000000a2", "spotify:track:a", &[(RECIPE, "second")]),
                item("", "spotify:track:c", &[(RECIPE, "third")]),
                item(
                    "00000000000000b2",
                    "spotify:track:b",
                    &[(ITEM_SPEED, "0.9")],
                ),
            ],
            &HashMap::new(),
        );
        assert_eq!(
            book.candidates(Some("00000000000000a2"), &["spotify:track:a"]),
            vec!["second"]
        );
        assert_eq!(
            book.candidates(Some("ffffffffffffffff"), &["spotify:track:a"]),
            vec!["first", "second"]
        );
        assert_eq!(
            book.candidates(None, &["spotify:track:x", "spotify:track:c"]),
            vec!["third"]
        );
        assert!(book.candidates(None, &["spotify:track:b"]).is_empty());
        assert!(book.fast(Some("00000000000000b2"), &[]));
        assert!(!book.fast(Some("00000000000000a1"), &["spotify:track:a"]));
    }

    #[test]
    fn reads_again_when_the_context_changes_or_goes_stale() {
        let now = Instant::now();
        let mut book = Book::default();
        assert!(book.wants_read("spotify:playlist:a", now));
        let version = book.start("spotify:playlist:a");
        assert_eq!(version, 1);
        assert!(!book.wants_read("spotify:playlist:a", now));
        assert!(book.wants_read("spotify:playlist:b", now));
        book.finish(Ok(Read::default()), now);
        assert_eq!(book.version, 2);
        assert!(!book.wants_read("spotify:playlist:a", now + FRESH / 2));
        assert!(book.wants_read("spotify:playlist:a", now + FRESH));

        book.start("spotify:playlist:a");
        book.finish(Err(anyhow!("offline")), now);
        assert_eq!(book.version, 2);
        assert_eq!(book.status, Some(Status::Failed));
        assert!(!book.wants_read("spotify:playlist:a", now + RETRY / 2));
        assert!(book.wants_read("spotify:playlist:a", now + RETRY));

        book.read.entries.push(Entry {
            uid: None,
            uri: "spotify:track:a".into(),
            recipe: "x".into(),
        });
        assert_eq!(book.start("spotify:playlist:b"), 3);
        assert!(book.read.entries.is_empty());
        assert_eq!(
            book.describe(),
            "automix context=spotify:playlist:b status=loading mixed=false items=0 recipes=0"
        );
    }

    #[test]
    fn asks_for_one_page_at_a_time() {
        assert_eq!(
            page_path("00000000000000000000a1", 200),
            "/playlist/v2/playlist/00000000000000000000a1?decorate=revision,length,attributes&from=200&length=100"
        );
        let track = json!({"metadata": {"uri": "spotify:track:b", "linked_from_uri": "spotify:track:a", "item.speed": "1.2"}});
        assert_eq!(uris_of(&track), vec!["spotify:track:b", "spotify:track:a"]);
        assert_eq!(speed_of(&track), 1.2);
        assert_eq!(
            uris_of(&json!({"metadata": {"uri": "spotify:track:b", "linked_from_uri": ""}})),
            vec!["spotify:track:b"]
        );
    }

    fn recipe(b: &str, start_a_ms: i32, item_speed_b: f64) -> String {
        use base64::{Engine as _, engine::general_purpose::STANDARD};
        use prost::Message as _;
        use spotify_proto::automix::proto::{Overlap, Transition};
        STANDARD.encode(
            Transition {
                overlap: Some(Overlap {
                    track_a_uri: "spotify:track:a".into(),
                    track_b_uri: b.into(),
                    start_a_ms,
                    start_b_ms: 4_000,
                    duration_ms: 6_000,
                    item_speed_a: 1.,
                    item_speed_b,
                    ..Overlap::default()
                }),
                ..Transition::default()
            }
            .encode_to_vec(),
        )
    }

    #[test]
    fn takes_the_first_recipe_that_fits_and_plays() {
        let a = Side {
            uris: &["spotify:track:a"],
            uid: Some("00000000000000a1"),
        };
        let b = Side {
            uris: &["spotify:track:b"],
            uid: Some("00000000000000b2"),
        };
        let texts = vec![
            "garbage".to_owned(),
            recipe("spotify:track:other", 100_000, 1.),
            recipe("spotify:track:b", 100_000, 1.2),
            recipe("spotify:track:b", 199_500, 1.),
            recipe("spotify:track:b", 150_000, 1.),
            recipe("spotify:track:b", 120_000, 1.),
        ];
        let plan = first_plan(&texts, a, b, (200_000, 180_000)).unwrap();
        assert_eq!(
            (plan.at_ms, plan.start_b_ms, plan.length_ms),
            (150_000, 4_000, 6_000)
        );
        assert_eq!(first_plan(&texts[..4], a, b, (200_000, 180_000)), None);
        assert_eq!(first_plan(&[], a, b, (200_000, 180_000)), None);
    }

    #[test]
    fn starts_the_blend_at_the_recipe_point() {
        let plan = Plan {
            at_ms: 180_000,
            length_ms: 8_000,
            lead_in_ms: 1_000,
            start_b_ms: 12_000,
            shape: Shape::cut(),
            shortened: false,
        };
        assert_eq!(blend_at(&plan, 179_999), None);
        let blend = blend_at(&plan, 180_250).unwrap();
        assert_eq!(
            (blend.length_ms, blend.lead_in_ms, blend.elapsed_ms),
            (8_000, 1_000, 250)
        );
        assert_eq!(blend.shape, Shape::cut());
        assert!(blend_at(&plan, 184_000).is_some());
        assert_eq!(blend_at(&plan, 184_001), None);
    }
}
