use std::time::Duration;

use spotify_client::{Api, dealer::Message, library, pathfinder::PathfinderError};
use tokio::time::Instant;

use super::{Ctx, Mutation, User};

const WINDOW: Duration = Duration::from_millis(100);

pub enum Command {
    Lookup(Vec<String>),
    Set { uris: Vec<String>, saved: bool },
}

pub enum Event {
    Known {
        found: Vec<(String, Option<bool>)>,
        asked: Option<std::time::Instant>,
    },
    Written {
        uris: Vec<String>,
        saved: bool,
    },
}

#[derive(Default)]
pub struct Lookups {
    uris: Vec<String>,
    due: Option<Instant>,
}

impl Lookups {
    pub fn pending(&self) -> bool {
        self.due.is_some()
    }

    pub async fn wait(&self) {
        match self.due {
            Some(due) => tokio::time::sleep_until(due).await,
            None => std::future::pending().await,
        }
    }
}

pub fn handle(command: Command, ctx: &mut Ctx) {
    match command {
        Command::Lookup(uris) => {
            for uri in uris {
                if library::savable(&uri) && !ctx.saved.uris.contains(&uri) {
                    ctx.saved.uris.push(uri);
                }
            }
            if !ctx.saved.uris.is_empty() && ctx.saved.due.is_none() {
                ctx.saved.due = Some(Instant::now() + WINDOW);
            }
        }
        Command::Set { uris, saved } => {
            let (api, user) = (ctx.api.clone(), ctx.user.clone());
            ctx.writes.push(Mutation::new(uris.clone(), async move {
                match save(&api, &user, &uris, saved).await {
                    Ok(()) => Ok(Event::Written { uris, saved }.into()),
                    Err(_) if saved => Err("Couldn’t add this to your library. Try again.".into()),
                    Err(_) => Err("Couldn’t remove this from your library. Try again.".into()),
                }
            }));
        }
    }
}

pub fn flush(ctx: &mut Ctx) {
    ctx.saved.due = None;
    let uris = std::mem::take(&mut ctx.saved.uris);
    let asked = Some(std::time::Instant::now());
    for chunk in uris.chunks(library::BATCH) {
        let (api, user) = (ctx.api.clone(), ctx.user.clone());
        let chunk = chunk.to_vec();
        ctx.tasks.spawn(async move {
            let found = contains(&api, &user, &chunk)
                .await
                .unwrap_or_else(|_| vec![None; chunk.len()]);
            Event::Known {
                found: chunk.into_iter().zip(found).collect(),
                asked,
            }
            .into()
        });
    }
}

async fn contains(api: &Api, user: &User, uris: &[String]) -> anyhow::Result<Vec<Option<bool>>> {
    match library::contains(api, uris).await {
        Err(error) if library::wants_fallback(&error) => {
            library::contains_in_sets(api, &user.name(api).await?, uris).await
        }
        found => found,
    }
}

async fn save(api: &Api, user: &User, uris: &[String], saved: bool) -> anyhow::Result<()> {
    match library::save(api, uris, saved).await {
        Err(error) if stale(&error) => {
            library::save_in_sets(api, &user.name(api).await?, uris, saved).await
        }
        result => result,
    }
}

fn stale(error: &anyhow::Error) -> bool {
    matches!(
        error.downcast_ref::<PathfinderError>(),
        Some(PathfinderError::StaleHash(_))
    )
}

pub fn on_push(message: &Message, ctx: &mut Ctx) {
    if let Some(changes) = library::collection_push(message)
        && !changes.is_empty()
    {
        let found = changes
            .into_iter()
            .map(|(uri, saved)| (uri, Some(saved)))
            .collect();
        ctx.publish(Event::Known { found, asked: None });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_stale_hash_sends_a_save_through_collection2() {
        let stale_hash = anyhow::Error::new(PathfinderError::StaleHash("addToLibrary"));
        let refused = anyhow::Error::new(PathfinderError::GraphQl(vec!["refused".into()]));
        assert!(stale(&stale_hash));
        assert!(!stale(&refused));
        assert!(!stale(&anyhow::anyhow!("offline")));
        assert!(library::wants_fallback(&stale_hash));
        assert!(library::wants_fallback(&refused));
        assert!(!library::wants_fallback(&anyhow::anyhow!("offline")));
    }
}
