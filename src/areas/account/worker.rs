use std::time::Duration;

use spotify_client::{
    Api,
    account::{self, AccountInfo},
    dealer::Message,
};

use super::state::{AccountPrefs, chosen_language, system_language};
use crate::worker::Ctx;

const SECOND_LOOK: Duration = Duration::from_secs(5);

pub enum Command {
    Refresh,
    Language(Option<String>),
}

pub enum Event {
    Account(Result<AccountInfo, String>),
}

impl Event {
    pub fn covers(&self) -> Vec<String> {
        match self {
            Self::Account(Ok(info)) => info.avatar.iter().cloned().collect(),
            Self::Account(Err(_)) => Vec::new(),
        }
    }
}

pub fn handle(command: Command, ctx: &mut Ctx) {
    match command {
        Command::Refresh => refresh(ctx),
        Command::Language(language) => apply_language(language.as_deref(), ctx),
    }
}

pub fn on_push(message: &Message, ctx: &mut Ctx) {
    if account::attributes_changed(&message.uri) {
        refresh(ctx);
        ctx.tasks
            .spawn_keyed("account-later", fetch(ctx.api.clone(), Some(SECOND_LOOK)));
    }
}

pub fn apply_settings(prefs: &AccountPrefs, ctx: &mut Ctx) {
    apply_language(prefs.language.as_deref(), ctx);
}

fn apply_language(language: Option<&str>, ctx: &mut Ctx) {
    let language = chosen_language(language, system_language);
    if ctx.api.language() != language {
        ctx.api.set_language(language);
    }
}

fn refresh(ctx: &mut Ctx) {
    ctx.tasks
        .spawn_keyed("account", fetch(ctx.api.clone(), None));
}

async fn fetch(api: Api, delay: Option<Duration>) -> crate::worker::Event {
    if let Some(delay) = delay {
        tokio::time::sleep(delay).await;
    }
    let result = account::account(&api)
        .await
        .map_err(|_| "Couldn’t load your account.".to_owned());
    Event::Account(result).into()
}
