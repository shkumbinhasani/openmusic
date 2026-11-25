use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

pub const RESOLVE_LIMIT: Duration = Duration::from_secs(30);
pub const SETTLE: Duration = Duration::from_secs(3);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ready {
    pub next: bool,
    pub uris: Option<Vec<String>>,
}

#[derive(Debug)]
enum Songs {
    Resolving(Instant),
    Ready(Option<Vec<String>>),
}

#[derive(Debug)]
struct Entry {
    id: u64,
    next: bool,
    songs: Songs,
}

#[derive(Debug, Default)]
pub struct QueueOrder {
    issued: u64,
    waiting: VecDeque<Entry>,
    sent: Option<(Option<String>, Instant)>,
}

impl QueueOrder {
    pub fn busy(&self) -> bool {
        !self.waiting.is_empty()
    }

    pub fn resolving(&mut self, next: bool, now: Instant) -> u64 {
        self.push(next, Songs::Resolving(now))
    }

    pub fn hold(&mut self, uris: Vec<String>, next: bool) {
        self.push(next, Songs::Ready(Some(uris)));
    }

    fn push(&mut self, next: bool, songs: Songs) -> u64 {
        self.issued += 1;
        self.waiting.push_back(Entry {
            id: self.issued,
            next,
            songs,
        });
        self.issued
    }

    pub fn resolved(&mut self, id: u64, uris: Option<Vec<String>>) {
        if let Some(entry) = self
            .waiting
            .iter_mut()
            .find(|entry| entry.id == id && matches!(entry.songs, Songs::Resolving(_)))
        {
            entry.songs = Songs::Ready(uris);
        }
    }

    pub fn due(&mut self, now: Instant, revision: Option<&str>) -> Option<Ready> {
        if let Some((before, at)) = &self.sent {
            let moved = revision.is_some() && before.as_deref() != revision;
            if !moved && now.duration_since(*at) < SETTLE {
                return None;
            }
        }
        let entry = self.waiting.front_mut()?;
        let uris = match &mut entry.songs {
            Songs::Ready(uris) => uris.take(),
            Songs::Resolving(since) if now.duration_since(*since) >= RESOLVE_LIMIT => None,
            Songs::Resolving(_) => return None,
        };
        let next = entry.next;
        self.waiting.pop_front();
        self.sent = uris
            .as_ref()
            .filter(|uris| !uris.is_empty())
            .map(|_| (revision.map(str::to_owned), now));
        Some(Ready { next, uris })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn songs(uris: &[&str]) -> Option<Vec<String>> {
        Some(uris.iter().map(|uri| (*uri).to_owned()).collect())
    }

    fn ready(next: bool, uris: &[&str]) -> Option<Ready> {
        Some(Ready {
            next,
            uris: songs(uris),
        })
    }

    #[test]
    fn hands_out_resolved_contexts_in_the_order_they_were_asked_for() {
        let now = Instant::now();
        let mut order = QueueOrder::default();
        assert!(!order.busy());
        let first = order.resolving(false, now);
        let second = order.resolving(true, now);
        order.hold(songs(&["t"]).unwrap(), false);
        order.resolved(second, songs(&["b1", "b2"]));
        assert_eq!(order.due(now, Some("1")), None);
        assert!(order.busy());
        order.resolved(first, songs(&["a1", "a2"]));
        assert_eq!(order.due(now, Some("1")), ready(false, &["a1", "a2"]));
        assert_eq!(order.due(now, Some("1")), None);
        assert_eq!(order.due(now, Some("2")), ready(true, &["b1", "b2"]));
        assert_eq!(order.due(now + SETTLE, Some("2")), ready(false, &["t"]));
        assert_eq!(order.due(now + SETTLE * 2, Some("3")), None);
        assert!(!order.busy());
    }

    #[test]
    fn moves_on_at_once_past_a_context_with_nothing_to_add() {
        let now = Instant::now();
        let mut order = QueueOrder::default();
        let empty = order.resolving(false, now);
        let failed = order.resolving(false, now);
        order.hold(songs(&["t"]).unwrap(), true);
        order.resolved(empty, songs(&[]));
        order.resolved(failed, None);
        assert_eq!(order.due(now, None), ready(false, &[]));
        assert_eq!(
            order.due(now, None),
            Some(Ready {
                next: false,
                uris: None
            })
        );
        assert_eq!(order.due(now, None), ready(true, &["t"]));
    }

    #[test]
    fn gives_up_on_a_context_that_never_resolves() {
        let start = Instant::now();
        let mut order = QueueOrder::default();
        let stuck = order.resolving(false, start);
        let later = order.resolving(false, start);
        order.resolved(later, songs(&["b"]));
        assert_eq!(order.due(start + RESOLVE_LIMIT / 2, None), None);
        assert_eq!(
            order.due(start + RESOLVE_LIMIT, None),
            Some(Ready {
                next: false,
                uris: None
            })
        );
        assert_eq!(order.due(start + RESOLVE_LIMIT, None), ready(false, &["b"]));
        order.resolved(stuck, songs(&["a"]));
        assert!(!order.busy());
    }
}
