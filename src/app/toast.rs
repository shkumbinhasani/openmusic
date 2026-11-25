use std::{
    rc::Rc,
    time::{Duration, Instant},
};

use gpui::Context;

use crate::app::OpenMusic;

const SHOWN_FOR: Duration = Duration::from_secs(4);
const SHOWN_WITH_UNDO_FOR: Duration = Duration::from_secs(6);

pub type Undo = Rc<dyn Fn(&mut OpenMusic, &mut Context<OpenMusic>)>;

pub struct Toast {
    pub text: String,
    pub undo: Option<Undo>,
    pub until: Instant,
}

impl OpenMusic {
    pub fn show_toast(&mut self, text: impl Into<String>, undo: Option<Undo>) {
        let shown_for = if undo.is_some() {
            SHOWN_WITH_UNDO_FOR
        } else {
            SHOWN_FOR
        };
        self.toast = Some(Toast {
            text: text.into(),
            undo,
            until: Instant::now() + shown_for,
        });
    }

    pub fn show_toast_if_idle(&mut self, text: impl Into<String>) {
        let idle = self
            .toast
            .as_ref()
            .is_none_or(|toast| toast.until <= Instant::now());
        if idle {
            self.show_toast(text, None);
        }
    }

    pub fn undo_toast(&mut self, cx: &mut Context<Self>) {
        if let Some(undo) = self.toast.take().and_then(|toast| toast.undo) {
            undo(self, cx);
        }
        cx.notify();
    }

    pub(super) fn expire_toast(&mut self) -> bool {
        if self
            .toast
            .as_ref()
            .is_some_and(|toast| toast.until <= Instant::now())
        {
            self.toast = None;
            return true;
        }
        false
    }
}
