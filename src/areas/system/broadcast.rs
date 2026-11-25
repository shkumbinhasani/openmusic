use std::time::Instant;

use serde_json::{Map, Value, json};

use super::now_playing::{Snapshot, should_publish};

pub use native::{listen, post, received};

pub const NAME: &str = "io.openmusic.player.PlaybackStateChanged";
pub const SENDER: &str = "io.openmusic.player";

#[derive(Clone, Debug, PartialEq)]
pub enum Field {
    Text(String),
    Number(f64),
}

impl Field {
    pub fn json(&self) -> Value {
        match self {
            Field::Text(text) => json!(text),
            Field::Number(number) => json!(number),
        }
    }
}

pub fn player_state(snapshot: Option<&Snapshot>) -> &'static str {
    match snapshot {
        None => "Stopped",
        Some(snapshot) if snapshot.playing => "Playing",
        Some(_) => "Paused",
    }
}

pub fn payload(snapshot: Option<&Snapshot>) -> Vec<(&'static str, Field)> {
    let mut fields = vec![(
        "Player State",
        Field::Text(player_state(snapshot).to_owned()),
    )];
    let Some(snapshot) = snapshot else {
        return fields;
    };
    fields.push(("Track ID", Field::Text(snapshot.uri.clone())));
    fields.push(("Name", Field::Text(snapshot.title.clone())));
    if !snapshot.artist.is_empty() {
        fields.push(("Artist", Field::Text(snapshot.artist.clone())));
    }
    if !snapshot.album.is_empty() {
        fields.push(("Album", Field::Text(snapshot.album.clone())));
    }
    if snapshot.duration_ms > 0 {
        fields.push(("Duration", Field::Number(f64::from(snapshot.duration_ms))));
    }
    fields.push((
        "Playback Position",
        Field::Number(f64::from(snapshot.position_ms) / 1000.),
    ));
    fields
}

pub fn describe(fields: &[(&'static str, Field)]) -> Value {
    Value::Object(
        fields
            .iter()
            .map(|(key, field)| ((*key).to_owned(), field.json()))
            .collect::<Map<_, _>>(),
    )
}

#[derive(Debug)]
enum Posted {
    Stopped,
    Playing(Snapshot, Instant),
}

#[derive(Debug, Default)]
pub struct Broadcaster {
    last: Option<Posted>,
    pub posts: u64,
}

impl Broadcaster {
    pub fn observe(
        &mut self,
        next: Option<Snapshot>,
        enabled: bool,
        now: Instant,
    ) -> Option<Option<Snapshot>> {
        if !enabled {
            self.last = None;
            return None;
        }
        let next = next.map(|snapshot| Snapshot {
            cover: None,
            ..snapshot
        });
        match (next, &self.last) {
            (Some(next), _) if next.title.is_empty() => None,
            (None, Some(Posted::Stopped)) => None,
            (None, _) => {
                self.last = Some(Posted::Stopped);
                self.posts += 1;
                Some(None)
            }
            (Some(next), last) => {
                if let Some(Posted::Playing(last, at)) = last {
                    let elapsed = now.saturating_duration_since(*at).as_millis() as u64;
                    if !should_publish(Some(last), &next, elapsed) {
                        return None;
                    }
                }
                self.last = Some(Posted::Playing(next.clone(), now));
                self.posts += 1;
                Some(Some(next))
            }
        }
    }

    pub fn last(&self) -> Option<Option<&Snapshot>> {
        self.last.as_ref().map(|posted| match posted {
            Posted::Stopped => None,
            Posted::Playing(snapshot, _) => Some(snapshot),
        })
    }
}

#[cfg(target_os = "macos")]
mod native {
    use std::{cell::RefCell, ptr::NonNull};

    use block2::RcBlock;
    use objc2::{
        rc::Retained,
        runtime::{AnyObject, ProtocolObject},
    };
    use objc2_foundation::{
        NSDictionary, NSDistributedNotificationCenter, NSMutableDictionary, NSNotification,
        NSNumber, NSObjectProtocol, NSOperationQueue, NSString,
    };
    use serde_json::{Map, Value, json};

    use super::{Field, NAME, SENDER};

    struct Listener {
        _token: Retained<ProtocolObject<dyn NSObjectProtocol>>,
        received: u64,
        last: Option<Value>,
    }

    thread_local! {
        static LISTENER: RefCell<Option<Listener>> = RefCell::default();
    }

    pub fn post(fields: &[(&'static str, Field)]) {
        let info: Retained<NSMutableDictionary<NSString, AnyObject>> = NSMutableDictionary::new();
        for (key, field) in fields {
            let key = NSString::from_str(key);
            let value: Retained<AnyObject> = match field {
                Field::Text(text) => NSString::from_str(text).into(),
                Field::Number(number) => NSNumber::new_f64(*number).into(),
            };
            info.insert(&*key, &*value);
        }
        let info: Retained<NSDictionary> = unsafe { Retained::cast_unchecked(info) };
        unsafe {
            NSDistributedNotificationCenter::defaultCenter()
                .postNotificationName_object_userInfo_deliverImmediately(
                    &NSString::from_str(NAME),
                    Some(&NSString::from_str(SENDER)),
                    Some(&info),
                    true,
                );
        }
    }

    fn value(object: &AnyObject) -> Value {
        if let Some(text) = object.downcast_ref::<NSString>() {
            json!(text.to_string())
        } else if let Some(number) = object.downcast_ref::<NSNumber>() {
            json!(number.doubleValue())
        } else {
            Value::Null
        }
    }

    fn record(notification: &NSNotification) {
        let mut fields = Map::new();
        if let Some(info) = notification.userInfo() {
            let (keys, objects) = info.to_vecs();
            for (key, object) in keys.iter().zip(objects.iter()) {
                let key = key
                    .downcast_ref::<NSString>()
                    .map(|key| key.to_string())
                    .unwrap_or_default();
                fields.insert(key, value(object));
            }
        }
        LISTENER.with_borrow_mut(|listener| {
            if let Some(listener) = listener {
                listener.received += 1;
                listener.last = Some(Value::Object(fields));
            }
        });
    }

    pub fn listen() {
        if LISTENER.with_borrow(Option::is_some) {
            return;
        }
        let block = RcBlock::new(|notification: NonNull<NSNotification>| {
            record(unsafe { notification.as_ref() })
        });
        let token = unsafe {
            NSDistributedNotificationCenter::defaultCenter()
                .addObserverForName_object_queue_usingBlock(
                    Some(&NSString::from_str(NAME)),
                    None,
                    Some(&NSOperationQueue::mainQueue()),
                    &block,
                )
        };
        LISTENER.set(Some(Listener {
            _token: token,
            received: 0,
            last: None,
        }));
    }

    pub fn received() -> Option<(u64, Option<Value>)> {
        LISTENER.with_borrow(|listener| {
            listener
                .as_ref()
                .map(|listener| (listener.received, listener.last.clone()))
        })
    }
}

#[cfg(not(target_os = "macos"))]
mod native {
    use serde_json::Value;

    use super::Field;

    pub fn post(_: &[(&'static str, Field)]) {}

    pub fn listen() {}

    pub fn received() -> Option<(u64, Option<Value>)> {
        None
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn snapshot(uri: &str, playing: bool, position_ms: u32) -> Snapshot {
        Snapshot {
            uri: uri.into(),
            title: "Title".into(),
            artist: "Artist".into(),
            album: "Album".into(),
            duration_ms: 200_000,
            position_ms,
            playing,
            speed: 1.,
            cover: Some("https://i.scdn.co/image/ab67616d0000b273000000000000000000000000".into()),
        }
    }

    const FIRST: &str = "spotify:track:4uLU6hMCjMI75M1A2tKUQC";
    const SECOND: &str = "spotify:track:1b5KESuOMQomF3wntoLAsk";

    #[test]
    fn the_payload_uses_spotifys_keys() {
        let playing = snapshot(FIRST, true, 61_500);
        let fields = payload(Some(&playing));
        assert_eq!(
            describe(&fields),
            json!({
                "Player State": "Playing",
                "Track ID": FIRST,
                "Name": "Title",
                "Artist": "Artist",
                "Album": "Album",
                "Duration": 200000.0,
                "Playback Position": 61.5,
            })
        );
        let paused = Snapshot {
            artist: String::new(),
            album: String::new(),
            duration_ms: 0,
            ..snapshot(FIRST, false, 0)
        };
        assert_eq!(
            describe(&payload(Some(&paused))),
            json!({
                "Player State": "Paused",
                "Track ID": FIRST,
                "Name": "Title",
                "Playback Position": 0.0,
            })
        );
        assert_eq!(
            describe(&payload(None)),
            json!({ "Player State": "Stopped" })
        );
    }

    #[test]
    fn posts_on_track_state_and_seek_changes_only() {
        let start = Instant::now();
        let mut broadcaster = Broadcaster::default();
        let first = broadcaster.observe(Some(snapshot(FIRST, true, 0)), true, start);
        assert_eq!(first.flatten().map(|posted| posted.cover), Some(None));
        let later = start + Duration::from_secs(5);
        assert_eq!(
            broadcaster.observe(Some(snapshot(FIRST, true, 5_000)), true, later),
            None
        );
        assert!(
            broadcaster
                .observe(Some(snapshot(FIRST, false, 5_000)), true, later)
                .is_some()
        );
        let seek = start + Duration::from_secs(6);
        assert!(
            broadcaster
                .observe(Some(snapshot(FIRST, false, 90_000)), true, seek)
                .is_some()
        );
        assert!(
            broadcaster
                .observe(Some(snapshot(SECOND, true, 0)), true, seek)
                .is_some()
        );
        assert_eq!(broadcaster.posts, 4);
    }

    #[test]
    fn a_new_cover_alone_does_not_post() {
        let start = Instant::now();
        let mut broadcaster = Broadcaster::default();
        let bare = Snapshot {
            cover: None,
            ..snapshot(FIRST, true, 0)
        };
        assert!(broadcaster.observe(Some(bare), true, start).is_some());
        assert_eq!(
            broadcaster.observe(Some(snapshot(FIRST, true, 0)), true, start),
            None
        );
    }

    #[test]
    fn waits_for_the_title_and_posts_stopped_once() {
        let start = Instant::now();
        let mut broadcaster = Broadcaster::default();
        let loading = Snapshot {
            title: String::new(),
            ..snapshot(FIRST, true, 0)
        };
        assert_eq!(broadcaster.observe(Some(loading), true, start), None);
        assert_eq!(broadcaster.observe(None, true, start), Some(None));
        assert_eq!(broadcaster.observe(None, true, start), None);
        assert!(
            broadcaster
                .observe(Some(snapshot(FIRST, true, 0)), true, start)
                .is_some()
        );
        let loading = Snapshot {
            title: String::new(),
            ..snapshot(SECOND, true, 0)
        };
        assert_eq!(broadcaster.observe(Some(loading), true, start), None);
        assert_eq!(broadcaster.observe(None, true, start), Some(None));
        assert_eq!(broadcaster.last(), Some(None));
    }

    #[test]
    fn turning_off_posts_nothing_and_turning_on_posts_at_once() {
        let start = Instant::now();
        let mut broadcaster = Broadcaster::default();
        assert_eq!(
            broadcaster.observe(Some(snapshot(FIRST, true, 0)), false, start),
            None
        );
        assert_eq!(broadcaster.posts, 0);
        assert!(
            broadcaster
                .observe(Some(snapshot(FIRST, true, 0)), true, start)
                .is_some()
        );
        assert_eq!(
            broadcaster.observe(Some(snapshot(FIRST, true, 0)), false, start),
            None
        );
        assert_eq!(broadcaster.last(), None);
        assert!(
            broadcaster
                .observe(Some(snapshot(FIRST, true, 0)), true, start)
                .is_some()
        );
    }
}
