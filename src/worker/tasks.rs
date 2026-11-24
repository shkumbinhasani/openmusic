use std::{collections::HashMap, future::Future, pin::Pin};

use tokio::{
    sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel},
    task::{AbortHandle, JoinSet},
};

use super::Event;

#[derive(Default)]
pub struct Tasks {
    running: JoinSet<Event>,
    keyed: HashMap<String, AbortHandle>,
}

impl Tasks {
    pub fn spawn(&mut self, task: impl Future<Output = Event> + Send + 'static) {
        self.running.spawn(task);
    }

    pub fn spawn_keyed(
        &mut self,
        key: impl Into<String>,
        task: impl Future<Output = Event> + Send + 'static,
    ) {
        self.keyed.retain(|_, handle| !handle.is_finished());
        let handle = self.running.spawn(task);
        if let Some(earlier) = self.keyed.insert(key.into(), handle) {
            earlier.abort();
        }
    }

    pub fn is_empty(&self) -> bool {
        self.running.is_empty()
    }

    pub async fn next(&mut self) -> Option<Event> {
        loop {
            match self.running.join_next().await? {
                Ok(event) => return Some(event),
                Err(_) => continue,
            }
        }
    }
}

type Run = Pin<Box<dyn Future<Output = Result<Event, String>> + Send>>;

pub struct Mutation {
    keys: Vec<String>,
    run: Run,
}

impl Mutation {
    pub fn new(
        keys: Vec<String>,
        run: impl Future<Output = Result<Event, String>> + Send + 'static,
    ) -> Self {
        Self {
            keys,
            run: Box::pin(run),
        }
    }
}

pub struct Writes {
    queue: UnboundedSender<Mutation>,
    done: UnboundedReceiver<Event>,
    writer: AbortHandle,
}

impl Writes {
    pub fn start() -> Self {
        let (queue, mut jobs) = unbounded_channel::<Mutation>();
        let (finished, done) = unbounded_channel();
        let writer = tokio::spawn(async move {
            while let Some(Mutation { keys, run }) = jobs.recv().await {
                let event = run
                    .await
                    .unwrap_or_else(|message| Event::MutationFailed { keys, message });
                if finished.send(event).is_err() {
                    break;
                }
            }
        })
        .abort_handle();
        Self {
            queue,
            done,
            writer,
        }
    }

    pub fn push(&self, mutation: Mutation) {
        let _ = self.queue.send(mutation);
    }

    pub async fn next(&mut self) -> Option<Event> {
        self.done.recv().await
    }
}

impl Drop for Writes {
    fn drop(&mut self) {
        self.writer.abort();
    }
}
