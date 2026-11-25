use std::{
    collections::{HashMap, HashSet},
    time::{Duration, Instant},
};

use gpui::{
    AnyElement, ClipboardItem, Context, Div, FontWeight, SharedString, Window, div, prelude::*, px,
    rgb, rgba,
};
use gpui_component::{
    ActiveTheme, Disableable, WindowExt, button::ButtonVariant, dialog::DialogButtonProps,
    switch::Switch,
};
use spotify_client::{
    Track,
    connect::ConnectDevice,
    jam::{self, Change, JoinMode, Member, Person, Session, SessionUpdate, VolumeControl},
};

use super::{
    state::{PickerAction, PickerRow, QUEUE_CURRENT, QUEUE_QUEUED},
    view::{heading, jump},
    worker::{Command, JamAction, JamResult},
};
use crate::{
    app::{OpenMusic, Page},
    icons,
    ui::{
        Palette,
        cards::Face,
        components::{ROW_RADIUS, icon},
        tracks::RowOptions,
    },
};

pub const JAM_POLL: Duration = Duration::from_secs(30);
const JAM_PENDING: Duration = Duration::from_secs(15);
const JAM_SETTLE: Duration = Duration::from_secs(5);
pub const LINK_COPIED: &str = "Link copied to clipboard";
pub const JAM_STARTED: &str = "Your Jam has started.";
pub const JAM_ENDED: &str = "The Jam has ended.";
pub const LEAVE_FIRST: &str = "Leave the Jam to play on another device.";
pub const GUEST_WARNING: &str =
    "Only the host can do this. You can right-click a song to add it to the queue.";
pub const HOST_VOLUME: &str = "Only the host can change the speaker volume";
pub const JAM_SYMBOL: &str = "person.2.wave.2";

pub const PAGE_START: &str = "jam-page-start";
pub const PAGE_INVITE: &str = "jam-page-invite";
pub const PAGE_LEAVE: &str = "jam-page-leave";
pub const PAGE_END: &str = "jam-page-end";
pub const PANEL_INVITE: &str = "jam-panel-invite";
pub const PANEL_LEAVE: &str = "jam-panel-leave";
pub const PANEL_END: &str = "jam-panel-end";
pub const JOIN: &str = "jam-join";
pub const NOT_NOW: &str = "jam-not-now";
pub const OPEN: &str = "jam-open";

#[derive(Clone, Debug, PartialEq)]
pub struct Preview {
    pub token: String,
    pub result: Option<Result<Option<Session>, String>>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PlayAfter {
    Track(Box<Track>),
    Context(String),
}

#[derive(Default)]
pub struct JamState {
    pub session: Option<Session>,
    pub preview: Option<Preview>,
    pub pending: Option<(JamAction, Instant)>,
    pub mode: JoinMode,
    pub people: HashMap<String, Person>,
    redirect: Option<String>,
    polled: Option<Instant>,
    play_after: Option<PlayAfter>,
    requested: HashSet<String>,
    departed: HashMap<String, i64>,
    noticed: bool,
    written: Option<Instant>,
}

impl JamState {
    pub fn covers(&self, urls: &mut HashSet<String>) {
        let previewed = self
            .preview
            .as_ref()
            .and_then(|preview| preview.result.as_ref())
            .and_then(|result| result.as_ref().ok())
            .and_then(Option::as_ref);
        for session in self.session.iter().chain(previewed) {
            urls.extend(
                session
                    .members
                    .iter()
                    .filter_map(|member| member.image.clone()),
            );
        }
        urls.extend(
            self.people
                .values()
                .filter_map(|person| person.image.clone()),
        );
    }

    pub fn redirect(&self) -> Option<&str> {
        self.redirect.as_deref()
    }
}

pub fn queue_only_guest(session: Option<&Session>) -> bool {
    session.is_some_and(|session| !session.is_owner && session.queue_only)
}

pub fn host_volume_only(session: Option<&Session>, active: Option<&ConnectDevice>) -> bool {
    let Some(session) = session else {
        return false;
    };
    let host_speaker = active.is_none_or(|device| !device.this_mac);
    let locked = !session.is_owner && session.volume_control != VolumeControl::Enabled;
    let unaddressable = session.redirect_commands && active.is_none();
    (locked && host_speaker) || unaddressable
}

pub fn people_label(count: usize) -> String {
    if count == 1 {
        "1 person".to_owned()
    } else {
        format!("{count} people")
    }
}

pub fn jam_title(session: &Session) -> String {
    format!("{}’s Jam", session.host_name())
}

pub fn faces(session: &Session) -> Vec<Face> {
    let mut members: Vec<&Member> = session.members.iter().collect();
    members.sort_by_key(|member| member.id != session.owner_id);
    members
        .into_iter()
        .map(|member| Face {
            name: member.display_name(),
            image: member.image.clone(),
        })
        .collect()
}

pub fn role(session: &Session, member: &Member) -> &'static str {
    if member.id == session.owner_id {
        "Host"
    } else if member.listening_along {
        "Listening along"
    } else {
        "Guest"
    }
}

pub fn jam_rows(session: Option<&Session>) -> Vec<PickerRow> {
    let mut rows = vec![PickerRow::Separator];
    let Some(session) = session else {
        rows.push(PickerRow::Action {
            action: PickerAction::StartJam,
            title: "Start a Jam".to_owned(),
            subtitle: Some("Listen with friends anywhere".to_owned()),
            symbol: JAM_SYMBOL,
        });
        return rows;
    };
    rows.push(PickerRow::Action {
        action: PickerAction::OpenJam,
        title: jam_title(session),
        subtitle: Some(people_label(session.members.len())),
        symbol: JAM_SYMBOL,
    });
    if session.can_invite && session.invite_url().is_some() {
        rows.push(PickerRow::Action {
            action: PickerAction::CopyJamLink,
            title: "Copy Jam link".to_owned(),
            subtitle: None,
            symbol: "link",
        });
    }
    rows.push(if session.is_owner {
        PickerRow::Action {
            action: PickerAction::EndJam,
            title: "End Jam".to_owned(),
            subtitle: None,
            symbol: "xmark.circle",
        }
    } else {
        PickerRow::Action {
            action: PickerAction::LeaveJam,
            title: "Leave Jam".to_owned(),
            subtitle: None,
            symbol: "rectangle.portrait.and.arrow.right",
        }
    });
    rows
}

pub fn queued_by(rows: &[&super::state::QueueRow]) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for name in rows.iter().filter_map(|row| row.queued_by.as_ref()) {
        if !names.contains(name) {
            names.push(name.clone());
        }
    }
    names
}

fn working(palette: Palette) -> Div {
    div()
        .text_size(px(12.))
        .text_color(rgb(palette.muted))
        .child("Working…")
}

fn confirm(
    window: &mut Window,
    cx: &mut Context<OpenMusic>,
    title: String,
    body: &'static str,
    ok: &'static str,
    run: impl Fn(&mut OpenMusic, &mut Context<OpenMusic>) + 'static,
) {
    let app = cx.entity().downgrade();
    let run = std::rc::Rc::new(run);
    window.open_dialog(cx, move |dialog, _, cx| {
        let app = app.clone();
        let run = run.clone();
        dialog
            .title(title.clone())
            .when(!body.is_empty(), |dialog| dialog.child(body))
            .w(px(380.))
            .bg(cx.theme().popover)
            .confirm()
            .button_props(
                DialogButtonProps::default()
                    .ok_text(ok)
                    .ok_variant(ButtonVariant::Danger)
                    .cancel_text("Cancel"),
            )
            .on_ok(move |_, _, cx| {
                let run = run.clone();
                let _ = app.update(cx, move |app, cx| run(app, cx));
                true
            })
    });
}

fn inform(window: &mut Window, cx: &mut Context<OpenMusic>, title: String, body: &'static str) {
    window.open_dialog(cx, move |dialog, _, cx| {
        dialog
            .title(title.clone())
            .child(body)
            .w(px(380.))
            .bg(cx.theme().popover)
            .alert()
            .button_props(DialogButtonProps::default().ok_text("Got it"))
    });
}

impl OpenMusic {
    pub fn jam(&self) -> Option<&Session> {
        self.playback.jam.session.as_ref()
    }

    pub fn jam_host(&self) -> bool {
        self.jam().is_some_and(|session| session.is_owner)
    }

    pub fn jam_guest(&self) -> bool {
        self.jam().is_some_and(|session| !session.is_owner)
    }

    pub fn jam_redirected(&self) -> bool {
        self.jam().is_some_and(|session| session.redirect_commands)
    }

    pub fn jam_controls_locked(&self) -> bool {
        queue_only_guest(self.jam())
    }

    pub fn jam_busy(&self) -> bool {
        self.playback
            .jam
            .pending
            .as_ref()
            .is_some_and(|(_, since)| since.elapsed() < JAM_PENDING)
    }

    pub fn refresh_jam(&mut self) {
        self.playback.jam.polled = Some(Instant::now());
        self.send(Command::JamRefresh);
    }

    fn write_jam(&mut self, action: JamAction, cx: &mut Context<Self>) {
        if self.jam_busy() {
            return;
        }
        self.playback.jam.pending = Some((action.clone(), Instant::now()));
        self.send(Command::JamWrite(action));
        cx.notify();
    }

    pub fn start_jam(&mut self, play: Option<PlayAfter>, cx: &mut Context<Self>) {
        if self.jam().is_some() {
            self.navigate(Page::Jam);
            cx.notify();
            return;
        }
        self.playback.jam.play_after = play;
        self.write_jam(JamAction::Start, cx);
    }

    pub fn join_jam(&mut self, token: String, mode: JoinMode, cx: &mut Context<Self>) {
        self.write_jam(JamAction::Join { token, mode }, cx);
    }

    pub fn leave_jam(&mut self, cx: &mut Context<Self>) {
        if let Some(id) = self.jam().map(|session| session.id.clone()) {
            self.write_jam(JamAction::Leave(id), cx);
        }
    }

    pub fn end_jam(&mut self, cx: &mut Context<Self>) {
        if let Some(id) = self
            .jam()
            .filter(|session| session.is_owner)
            .map(|session| session.id.clone())
        {
            self.write_jam(JamAction::End(id), cx);
        }
    }

    pub fn confirm_leave_jam(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.jam_guest() {
            return;
        }
        confirm(
            window,
            cx,
            "Are you sure you want to leave the Jam?".to_owned(),
            "You’ll no longer be listening together, and your queue will be cleared.",
            "Leave Jam",
            |app, cx| app.leave_jam(cx),
        );
    }

    pub fn confirm_end_jam(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.jam_host() {
            return;
        }
        confirm(
            window,
            cx,
            "Are you sure you want to end the Jam for everyone?".to_owned(),
            "You’ll no longer be listening together.",
            "End Jam",
            |app, cx| app.end_jam(cx),
        );
    }

    pub fn confirm_exit_jam(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.jam_host() {
            self.confirm_end_jam(window, cx);
        } else {
            self.confirm_leave_jam(window, cx);
        }
    }

    pub fn set_jam_queue_only(&mut self, on: bool, cx: &mut Context<Self>) {
        if self.jam().is_some_and(|session| session.can_manage) {
            self.write_jam(JamAction::QueueOnly(on), cx);
        }
    }

    pub fn set_jam_volume_control(&mut self, on: bool, cx: &mut Context<Self>) {
        if self.jam().is_some_and(|session| {
            session.can_manage && session.volume_control != VolumeControl::Unavailable
        }) {
            self.write_jam(JamAction::VolumeControl(on), cx);
        }
    }

    pub fn remove_from_jam(&mut self, member: String, cx: &mut Context<Self>) {
        let Some(session) = self
            .jam()
            .filter(|session| session.can_remove && session.owner_id != member)
        else {
            return;
        };
        let session = session.id.clone();
        self.write_jam(JamAction::Remove { session, member }, cx);
    }

    pub fn confirm_remove_from_jam(
        &mut self,
        member: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(name) = self
            .jam()
            .filter(|session| session.can_remove)
            .and_then(|session| {
                session
                    .members
                    .iter()
                    .find(|found| found.id == member && found.id != session.owner_id)
            })
            .map(Member::display_name)
        else {
            return;
        };
        let member = member.to_owned();
        confirm(
            window,
            cx,
            format!("Are you sure you want to remove {name} from the Jam?"),
            "",
            "Remove",
            move |app, cx| app.remove_from_jam(member.clone(), cx),
        );
    }

    pub fn copy_jam_link(&mut self, cx: &mut Context<Self>) {
        let Some(url) = self.jam().and_then(Session::invite_url) else {
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(url));
        self.show_toast(LINK_COPIED, None);
        cx.notify();
    }

    pub fn show_jam_fixture(&mut self, session: Option<Session>) {
        self.playback.jam.session = session.map(|session| Session {
            redirect_commands: false,
            ..session
        });
        self.playback.jam.polled = Some(Instant::now());
        self.request_people();
    }

    pub fn preview_jam(&mut self, token: &str) {
        let loaded = self.playback.jam.preview.as_ref().is_some_and(|preview| {
            preview.token == token && !matches!(preview.result, Some(Err(_)))
        });
        if loaded {
            return;
        }
        self.playback.jam.preview = Some(Preview {
            token: token.to_owned(),
            result: None,
        });
        self.send(Command::JamPreview(token.to_owned()));
    }

    fn sync_jam_redirect(&mut self) {
        let wanted = self
            .jam()
            .filter(|session| session.redirect_commands)
            .map(|session| session.id.clone());
        if wanted != self.playback.jam.redirect {
            self.playback.jam.redirect = wanted.clone();
            self.send(Command::JamRedirect(wanted));
        }
    }

    fn set_jam(
        &mut self,
        session: Option<Session>,
        reason: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let old = self.playback.jam.session.take();
        let owner = session
            .as_ref()
            .or(old.as_ref())
            .is_some_and(|session| session.is_owner);
        let changes = jam::changes(old.as_ref(), session.as_ref(), reason);
        if let Some(left) = old
            .as_ref()
            .filter(|old| session.as_ref().is_none_or(|session| session.id != old.id))
        {
            self.playback
                .jam
                .departed
                .insert(left.id.clone(), left.timestamp.max(1));
        }
        if let Some(joined) = &session {
            self.playback.jam.departed.remove(&joined.id);
        }
        self.playback.jam.session = session;
        if self.playback.jam.session.is_some() {
            self.playback.jam.polled = Some(Instant::now());
        }
        for change in changes {
            match change {
                Change::Ended { .. } if !owner => inform(
                    window,
                    cx,
                    "The host has ended the Jam".to_owned(),
                    "You’re no longer listening together, and the queue has been cleared.",
                ),
                change => {
                    if let Some(text) = change.message(owner) {
                        self.show_toast(text, None);
                    }
                }
            }
        }
        self.sync_jam_redirect();
        self.request_people();
    }

    fn play_after_start(&mut self, cx: &mut Context<Self>) {
        match self.playback.jam.play_after.take() {
            Some(PlayAfter::Track(track)) => self.play(&[*track], 0, cx),
            Some(PlayAfter::Context(uri)) => self.play_context(&uri, &[], 0, cx),
            None => {}
        }
    }

    pub fn request_people(&mut self) {
        if !self.in_jam() {
            return;
        }
        let queue = &self.playback.queue;
        let rows: Vec<&super::state::QueueRow> = queue
            .current
            .iter()
            .chain(&queue.queued)
            .chain(&queue.next)
            .collect();
        let ask: Vec<String> = queued_by(&rows)
            .into_iter()
            .filter(|name| {
                !self.playback.jam.people.contains_key(name)
                    && !self.playback.jam.requested.contains(name)
            })
            .collect();
        if ask.is_empty() {
            return;
        }
        self.playback.jam.requested.extend(ask.iter().cloned());
        self.send(Command::People(ask));
    }

    pub fn added_by(&self, username: &str) -> Face {
        if let Some(person) = self.playback.jam.people.get(username) {
            return Face {
                name: person.name.clone(),
                image: person.image.clone(),
            };
        }
        if let Some(member) = self.jam().and_then(|session| session.member(username)) {
            return Face {
                name: member.display_name(),
                image: member.image.clone(),
            };
        }
        Face {
            name: username.to_owned(),
            image: None,
        }
    }

    pub fn on_jam_update(
        &mut self,
        update: SessionUpdate,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let departed = self.playback.jam.departed.get(&update.session_id).copied();
        if jam::accepts(self.jam(), &update, departed) {
            self.set_jam(update.session, &update.reason, window, cx);
        }
    }

    pub fn on_jam_current(
        &mut self,
        result: JamResult,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Ok(found) = result else {
            return;
        };
        let departed = &self.playback.jam.departed;
        let settling = self
            .playback
            .jam
            .written
            .is_some_and(|at| at.elapsed() < JAM_SETTLE);
        match (found.map(|session| *session), self.jam()) {
            (Some(found), Some(current))
                if found.id == current.id && found.timestamp < current.timestamp => {}
            (Some(found), None)
                if departed
                    .get(&found.id)
                    .is_some_and(|left| found.timestamp <= *left) => {}
            (Some(found), _) => self.set_jam(Some(found), "UNKNOWN_UPDATE_TYPE", window, cx),
            (None, Some(_)) if settling => {}
            (None, Some(current)) => {
                let guest = !current.is_owner;
                self.set_jam(None, "YOU_LEFT", window, cx);
                if guest {
                    self.show_toast(JAM_ENDED, None);
                }
            }
            (None, None) => self.sync_jam_redirect(),
        }
    }

    pub fn on_jam_written(
        &mut self,
        action: JamAction,
        result: JamResult,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.playback.jam.pending = None;
        let session = match result {
            Err(message) => {
                if action == JamAction::Start {
                    self.playback.jam.play_after = None;
                }
                self.show_toast(message, None);
                return;
            }
            Ok(session) => session.map(|session| *session),
        };
        self.playback.jam.written = (!action.ends()).then(Instant::now);
        self.refresh_jam();
        if action.ends() {
            self.set_jam(None, action.reason(), window, cx);
            return;
        }
        let Some(session) = session else {
            let had = self.jam().is_some();
            self.set_jam(None, "YOU_LEFT", window, cx);
            if had || matches!(action, JamAction::Join { .. }) {
                self.show_toast(JAM_ENDED, None);
            }
            return;
        };
        match &action {
            JamAction::Start => {
                self.set_jam(Some(session), action.reason(), window, cx);
                self.show_toast(JAM_STARTED, None);
                self.play_after_start(cx);
                self.navigate(Page::Jam);
            }
            JamAction::Join { .. } => {
                let host = session.host_name();
                self.set_jam(Some(session), action.reason(), window, cx);
                self.show_toast(format!("You’re now sharing control of {host}’s Jam"), None);
                self.navigate(Page::Jam);
            }
            _ => self.set_jam(Some(session), action.reason(), window, cx),
        }
    }

    pub fn on_jam_preview(&mut self, token: String, result: JamResult) {
        if let Some(preview) = self
            .playback
            .jam
            .preview
            .as_mut()
            .filter(|preview| preview.token == token)
        {
            preview.result = Some(result.map(|session| session.map(|session| *session)));
        }
    }

    pub fn on_people(&mut self, asked: Vec<String>, found: Option<Vec<Person>>) {
        let Some(found) = found else {
            for name in &asked {
                self.playback.jam.requested.remove(name);
            }
            return;
        };
        for person in found {
            self.playback
                .jam
                .people
                .insert(person.username.clone(), person);
        }
        for name in asked {
            self.playback.jam.requested.insert(name);
        }
    }

    pub fn jam_tick(&mut self) -> bool {
        let mut changed = false;
        if self
            .playback
            .jam
            .pending
            .as_ref()
            .is_some_and(|(_, since)| since.elapsed() >= JAM_PENDING)
        {
            self.playback.jam.pending = None;
            changed = true;
        }
        let due = self
            .playback
            .jam
            .polled
            .is_none_or(|at| at.elapsed() >= JAM_POLL);
        if self.jam().is_some() && due {
            self.refresh_jam();
        }
        let failed = self.playback.notice.is_some();
        if self.playback.notice.as_deref() == Some(JAM_ENDED) {
            self.playback.jam.redirect = None;
        }
        if failed && self.jam_redirected() && !self.playback.jam.noticed {
            self.playback.jam.noticed = true;
            self.refresh_jam();
        } else if !failed {
            self.playback.jam.noticed = false;
        }
        changed
    }

    pub fn jam_page_body(&self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let Some(session) = self.jam() else {
            return self.jam_empty(palette, cx).into_any_element();
        };
        let busy = self.jam_busy();
        let subtitle = if session.queue_only && !session.is_owner {
            "Listen together in real time, from anywhere."
        } else {
            "Listen and add songs together in real time, from anywhere."
        };
        let actions = div()
            .flex()
            .items_center()
            .gap_3()
            .when(
                session.can_invite && session.invite_url().is_some(),
                |row| row.child(self.text_control(PAGE_INVITE, "Invite", true, palette, cx)),
            )
            .child(if session.is_owner {
                self.text_control(PAGE_END, "End Jam", false, palette, cx)
            } else {
                self.text_control(PAGE_LEAVE, "Leave Jam", false, palette, cx)
            })
            .when(busy, |row| row.child(working(palette)));
        let header = div()
            .flex()
            .items_center()
            .gap_4()
            .child(self.facepile(&faces(session), px(56.), 4, palette))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_size(px(30.))
                            .font_weight(FontWeight::BOLD)
                            .truncate()
                            .child(jam_title(session)),
                    )
                    .child(
                        div()
                            .text_size(px(13.))
                            .text_color(rgb(palette.muted))
                            .child(subtitle),
                    ),
            );
        let mut page = div().flex().flex_col().gap_6().child(header).child(actions);
        if session.paused || session.jam_paused {
            page = page.child(
                div()
                    .text_size(px(13.))
                    .text_color(rgb(palette.muted))
                    .child(format!("{} has paused the Jam…", session.host_name())),
            );
        }
        if session.can_manage {
            page = page.child(self.jam_settings(session, busy, palette, cx));
        }
        page.child(self.jam_members(session, palette, cx))
            .child(self.jam_queue(palette, cx))
            .into_any_element()
    }

    fn jam_empty(&self, palette: Palette, cx: &mut Context<Self>) -> Div {
        div()
            .flex()
            .flex_col()
            .items_center()
            .gap_3()
            .py_16()
            .child(icon(icons::PERSON, 40., palette.muted))
            .child(heading("Start a Jam").flex_none())
            .child(
                div()
                    .max_w(px(360.))
                    .text_center()
                    .text_size(px(13.))
                    .text_color(rgb(palette.muted))
                    .child(
                        "Invite friends to listen and add songs together in real time, from wherever they are.",
                    ),
            )
            .child(
                self.text_control(PAGE_START, "Start a Jam", true, palette, cx)
                    .mt_2(),
            )
            .when(self.jam_busy(), |page| page.child(working(palette)))
    }

    fn jam_settings(
        &self,
        session: &Session,
        busy: bool,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Div {
        let app = cx.entity().downgrade();
        let control = app.clone();
        let row = |label: &'static str, toggle: Switch| {
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap_4()
                .px_3()
                .py_2p5()
                .rounded(px(ROW_RADIUS))
                .bg(rgba(palette.press))
                .child(div().text_size(px(14.)).child(label))
                .child(toggle)
        };
        let mut settings = div().flex().flex_col().gap_2().child(row(
            "Let others change what’s playing",
            Switch::new("jam-queue-only")
                .checked(!session.queue_only)
                .disabled(busy)
                .on_click(move |checked, _, cx| {
                    let on = !*checked;
                    let _ = app.update(cx, move |app, cx| app.set_jam_queue_only(on, cx));
                }),
        ));
        if session.volume_control != VolumeControl::Unavailable {
            settings = settings.child(row(
                "Let others change the volume",
                Switch::new("jam-volume-control")
                    .checked(session.volume_control == VolumeControl::Enabled)
                    .disabled(busy)
                    .on_click(move |checked, _, cx| {
                        let on = *checked;
                        let _ =
                            control.update(cx, move |app, cx| app.set_jam_volume_control(on, cx));
                    }),
            ));
        }
        settings
    }

    fn jam_members(&self, session: &Session, palette: Palette, cx: &mut Context<Self>) -> Div {
        let mut members: Vec<&Member> = session
            .members
            .iter()
            .filter(|member| !member.anonymous)
            .collect();
        members.sort_by_key(|member| member.id != session.owner_id);
        let rows = members.into_iter().enumerate().map(|(index, member)| {
            let removable =
                session.can_remove && member.id != session.owner_id && !member.current_user;
            let id = member.id.clone();
            let name = if member.current_user {
                format!("{} (you)", member.display_name())
            } else {
                member.display_name()
            };
            div()
                .id(("jam-member", index))
                .flex()
                .items_center()
                .gap_3()
                .px_2()
                .py_1p5()
                .rounded(px(ROW_RADIUS))
                .hover(move |row| row.bg(rgba(palette.press)))
                .child(self.avatar(
                    &member.display_name(),
                    member.image.as_deref(),
                    px(40.),
                    palette,
                ))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .text_size(px(14.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .truncate()
                                .child(name),
                        )
                        .child(
                            div()
                                .text_size(px(12.))
                                .text_color(rgb(palette.muted))
                                .child(role(session, member)),
                        ),
                )
                .when(removable, |row| {
                    row.child(
                        div()
                            .id(("jam-remove", index))
                            .px_3()
                            .py_1()
                            .rounded_full()
                            .cursor_pointer()
                            .text_size(px(13.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(palette.muted))
                            .hover(move |button| {
                                button.bg(rgba(palette.fill)).text_color(rgb(palette.text))
                            })
                            .child("Remove")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.confirm_remove_from_jam(&id, window, cx);
                            })),
                    )
                })
        });
        let anonymous = session.anonymous_count();
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .min_h(px(36.))
                    .flex()
                    .items_center()
                    .px_2()
                    .child(heading("In this Jam")),
            )
            .children(rows)
            .when(anonymous > 0, |list| {
                list.child(
                    div()
                        .px_2()
                        .text_size(px(13.))
                        .text_color(rgb(palette.muted))
                        .child(if anonymous == 1 {
                            "1 anonymous guest".to_owned()
                        } else {
                            format!("{anonymous} anonymous guests")
                        }),
                )
            })
            .child(
                div()
                    .px_2()
                    .pt_1()
                    .text_size(px(12.))
                    .text_color(rgb(palette.muted))
                    .child("People in the Jam can see your username and profile picture."),
            )
    }

    fn jam_queue(&self, palette: Palette, cx: &mut Context<Self>) -> Div {
        let queue = &self.playback.queue;
        let base = RowOptions {
            context: self
                .playback_model()
                .and_then(|model| model.context_uri.clone()),
            ..RowOptions::default()
        };
        let mut list = div().flex().flex_col().gap_6();
        if queue.current.is_some() {
            let options = RowOptions {
                on_click: Some(jump(QUEUE_CURRENT)),
                ..base.clone()
            };
            list = list.child(self.queue_section(
                "Now playing",
                QUEUE_CURRENT,
                &options,
                None,
                palette,
                cx,
            ));
        }
        if queue.queued.is_empty() {
            return list.child(
                div()
                    .px_2()
                    .text_size(px(13.))
                    .text_color(rgb(palette.muted))
                    .child("Songs people add to the queue show here."),
            );
        }
        let options = RowOptions {
            on_click: Some(jump(QUEUE_QUEUED)),
            ..base
        };
        list.child(self.queue_section("Next in queue", QUEUE_QUEUED, &options, None, palette, cx))
    }

    pub fn jam_invite_body(
        &self,
        uri: &str,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let token = jam::token_from_uri(uri).unwrap_or_default();
        let preview = self
            .playback
            .jam
            .preview
            .as_ref()
            .filter(|preview| preview.token == token);
        let result = match preview.and_then(|preview| preview.result.as_ref()) {
            None => return self.page_status("Loading…", palette).into_any_element(),
            Some(Err(_)) => {
                return self
                    .page_status("Couldn’t load this Jam. Try again later.", palette)
                    .into_any_element();
            }
            Some(Ok(None)) => {
                return self
                    .page_status("This Jam has ended.", palette)
                    .into_any_element();
            }
            Some(Ok(Some(session))) => session,
        };
        let host = result.host_name();
        let joined = self.jam().is_some_and(|current| current.id == result.id);
        let heading_text: SharedString = if joined {
            format!("You’re in {host}’s Jam").into()
        } else {
            format!("Join {host}’s Jam").into()
        };
        let top = div()
            .flex()
            .flex_col()
            .items_center()
            .gap_3()
            .pt_8()
            .child(self.facepile(&faces(result), px(64.), 5, palette))
            .child(
                div()
                    .text_size(px(28.))
                    .font_weight(FontWeight::BOLD)
                    .text_center()
                    .child(heading_text),
            )
            .child(
                div()
                    .text_size(px(13.))
                    .text_color(rgb(palette.muted))
                    .child(people_label(result.members.len())),
            );
        if joined {
            return top
                .child(
                    self.text_control(OPEN, "Open Jam", true, palette, cx)
                        .mt_4(),
                )
                .into_any_element();
        }
        if !result.active {
            return top
                .child(self.page_status("This Jam has ended.", palette))
                .into_any_element();
        }
        let mode = self.playback.jam.mode;
        let choice = |id: &'static str,
                      choice: JoinMode,
                      title: String,
                      subtitle: &'static str,
                      cx: &mut Context<Self>| {
            let selected = mode == choice;
            div()
                .id(id)
                .w_full()
                .flex()
                .items_center()
                .gap_3()
                .px_4()
                .py_3()
                .rounded(px(ROW_RADIUS + 4.))
                .border_1()
                .border_color(rgba(if selected {
                    palette.accent << 8 | 0xff
                } else {
                    palette.rim
                }))
                .bg(rgba(if selected {
                    palette.fill
                } else {
                    palette.press
                }))
                .cursor_pointer()
                .hover(move |row| row.bg(rgba(palette.fill)))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .text_size(px(14.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(title),
                        )
                        .child(
                            div()
                                .text_size(px(12.))
                                .text_color(rgb(palette.muted))
                                .child(subtitle),
                        ),
                )
                .when(selected, |row| {
                    row.child(icon(icons::CHECK, 16., palette.accent))
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.playback.jam.mode = choice;
                    cx.notify();
                }))
        };
        let other = self
            .jam()
            .filter(|current| current.id != result.id)
            .map(jam_title);
        top.child(
            div()
                .w(px(420.))
                .max_w_full()
                .flex()
                .flex_col()
                .gap_2()
                .pt_4()
                .child(choice(
                    "jam-mode-remote",
                    JoinMode::Remote,
                    "Continue on This Mac".to_owned(),
                    "Choose if you’re joining remotely",
                    cx,
                ))
                .child(choice(
                    "jam-mode-person",
                    JoinMode::InPerson,
                    format!("Play from {host}’s speaker"),
                    "Choose if you’re joining in person",
                    cx,
                )),
        )
        .child(
            div()
                .max_w(px(420.))
                .text_center()
                .text_size(px(12.))
                .text_color(rgb(palette.muted))
                .child("People in the Jam will see your username and profile picture."),
        )
        .when_some(other, |page, other| {
            page.child(
                div()
                    .text_size(px(12.))
                    .text_color(rgb(palette.muted))
                    .child(format!("Joining takes you out of {other}.")),
            )
        })
        .child(
            div()
                .flex()
                .items_center()
                .gap_3()
                .pt_2()
                .child(self.text_control(NOT_NOW, "Not now", false, palette, cx))
                .child(self.text_control(JOIN, "Join", true, palette, cx)),
        )
        .when(self.jam_busy(), |page| page.child(working(palette)))
        .into_any_element()
    }

    pub fn jam_panel(&self, palette: Palette, cx: &mut Context<Self>) -> Option<AnyElement> {
        let session = self.jam()?;
        let summary = div()
            .id("jam-panel")
            .flex()
            .items_center()
            .gap_2()
            .cursor_pointer()
            .rounded(px(ROW_RADIUS))
            .hover(move |row| row.bg(rgba(palette.fill)))
            .child(self.facepile(&faces(session), px(22.), 3, palette))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .text_size(px(12.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .truncate()
                            .child(jam_title(session)),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(palette.muted))
                            .child(people_label(session.members.len())),
                    ),
            )
            .on_click(cx.listener(|this, _, _, cx| {
                cx.stop_propagation();
                this.navigate(Page::Jam);
                cx.notify();
            }));
        let buttons = div()
            .flex()
            .gap_2()
            .when(
                session.can_invite && session.invite_url().is_some(),
                |row| {
                    row.child(
                        self.text_control(PANEL_INVITE, "Invite", false, palette, cx)
                            .h(px(28.))
                            .flex_1(),
                    )
                },
            )
            .child(
                if session.is_owner {
                    self.text_control(PANEL_END, "End", false, palette, cx)
                } else {
                    self.text_control(PANEL_LEAVE, "Leave", false, palette, cx)
                }
                .h(px(28.))
                .flex_1(),
            );
        Some(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .pt_2()
                .border_t_1()
                .border_color(rgba(palette.line))
                .child(summary)
                .child(buttons)
                .into_any_element(),
        )
    }

    pub fn jam_queue_note(&self, palette: Palette, cx: &mut Context<Self>) -> Option<Div> {
        let session = self.jam()?;
        Some(
            div()
                .flex()
                .items_center()
                .gap_3()
                .px_2()
                .child(self.facepile(&faces(session), px(28.), 4, palette))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(px(13.))
                        .text_color(rgb(palette.muted))
                        .child(format!(
                            "You’re in {}. Everyone in the Jam shares this queue.",
                            jam_title(session)
                        )),
                )
                .child(
                    div()
                        .id("jam-queue-open")
                        .cursor_pointer()
                        .text_size(px(13.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(palette.accent))
                        .hover(|link| link.underline())
                        .child("Open Jam")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.navigate(Page::Jam);
                            cx.notify();
                        })),
                ),
        )
    }

    pub fn added_by_badge(&self, username: &str, palette: Palette) -> AnyElement {
        let face = self.added_by(username);
        div()
            .flex()
            .items_center()
            .gap_1p5()
            .max_w(px(140.))
            .flex_shrink_0()
            .child(self.avatar(&face.name, face.image.as_deref(), px(16.), palette))
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_size(px(12.))
                    .text_color(rgb(palette.muted))
                    .child(face.name),
            )
            .into_any_element()
    }
}

pub fn on_press(
    app: &mut OpenMusic,
    key: &'static str,
    window: &mut Window,
    cx: &mut Context<OpenMusic>,
) -> bool {
    match key {
        PAGE_START => app.start_jam(None, cx),
        PAGE_INVITE | PANEL_INVITE => app.copy_jam_link(cx),
        PAGE_LEAVE | PANEL_LEAVE | PAGE_END | PANEL_END => app.confirm_exit_jam(window, cx),
        OPEN => app.navigate(Page::Jam),
        NOT_NOW => app.back(),
        JOIN => {
            if let Page::JamInvite(uri) = &app.page
                && let Some(token) = jam::token_from_uri(uri)
            {
                let mode = app.playback.jam.mode;
                app.join_jam(token, mode, cx);
            }
        }
        _ => return false,
    }
    true
}

pub fn dump(app: &OpenMusic) -> String {
    let state = &app.playback.jam;
    let mut lines = vec![format!(
        "redirect={} pending={:?} mode={:?} people={} preview={}",
        state.redirect.as_deref().unwrap_or("-"),
        state.pending.as_ref().map(|(action, _)| action),
        state.mode,
        state.people.len(),
        match &state.preview {
            None => "-".to_owned(),
            Some(Preview {
                token,
                result: None,
            }) => format!("{token} loading"),
            Some(Preview {
                token,
                result: Some(Err(error)),
            }) => format!("{token} error={error:?}"),
            Some(Preview {
                token,
                result: Some(Ok(None)),
            }) => format!("{token} ended"),
            Some(Preview {
                token,
                result: Some(Ok(Some(session))),
            }) => format!(
                "{token} host={:?} members={} active={}",
                session.host_name(),
                session.members.len(),
                session.active
            ),
        },
    )];
    match app.jam() {
        None => lines.push("session=none".to_owned()),
        Some(session) => {
            lines.push(format!(
                "session={} timestamp={} owner={} host={:?} active={} queue_only={} volume={:?} redirect={} paused={} invite={:?} can_invite={} can_remove={} can_manage={} host_device={:?}",
                session.id,
                session.timestamp,
                session.is_owner,
                session.host_name(),
                session.active,
                session.queue_only,
                session.volume_control,
                session.redirect_commands,
                session.paused || session.jam_paused,
                session.invite_url(),
                session.can_invite,
                session.can_remove,
                session.can_manage,
                session.host_device_name,
            ));
            for member in &session.members {
                lines.push(format!(
                    "member {} name={:?} role={} you={} anonymous={} image={}",
                    member.id,
                    member.display_name(),
                    role(session, member),
                    member.current_user,
                    member.anonymous,
                    member
                        .image
                        .as_ref()
                        .is_some_and(|url| app.covers.contains_key(url)),
                ));
            }
        }
    }
    let queue = &app.playback.queue;
    for row in queue.current.iter().chain(&queue.queued).chain(&queue.next) {
        if let Some(name) = &row.queued_by {
            lines.push(format!(
                "added_by {} {} → {:?}",
                row.key.uid,
                name,
                app.added_by(name).name
            ));
        }
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(owner: bool) -> Session {
        Session {
            id: "s".into(),
            owner_id: "host".into(),
            is_owner: owner,
            active: true,
            join_uri: "spotify:socialsession:tok".into(),
            can_invite: true,
            members: vec![
                Member {
                    id: "guest".into(),
                    name: "Guest Person".into(),
                    listening_along: true,
                    ..Member::default()
                },
                Member {
                    id: "host".into(),
                    name: "Host Person".into(),
                    ..Member::default()
                },
            ],
            ..Session::default()
        }
    }

    fn action(row: &PickerRow) -> Option<PickerAction> {
        match row {
            PickerRow::Action { action, .. } => Some(*action),
            _ => None,
        }
    }

    #[test]
    fn lists_jam_rows_for_the_picker() {
        let idle = jam_rows(None);
        assert_eq!(idle[0], PickerRow::Separator);
        assert_eq!(action(&idle[1]), Some(PickerAction::StartJam));
        let hosting = jam_rows(Some(&session(true)));
        assert_eq!(
            hosting.iter().filter_map(action).collect::<Vec<_>>(),
            vec![
                PickerAction::OpenJam,
                PickerAction::CopyJamLink,
                PickerAction::EndJam
            ]
        );
        assert!(matches!(
            &hosting[1],
            PickerRow::Action { title, subtitle: Some(count), .. }
                if title == "Host Person’s Jam" && count == "2 people"
        ));
        let mut guest = session(false);
        guest.can_invite = false;
        assert_eq!(
            jam_rows(Some(&guest))
                .iter()
                .filter_map(action)
                .collect::<Vec<_>>(),
            vec![PickerAction::OpenJam, PickerAction::LeaveJam]
        );
    }

    #[test]
    fn collects_who_queued_songs_once() {
        let row = |uid: &str, by: Option<&str>| super::super::state::QueueRow {
            key: spotify_client::connect::QueueKey {
                uid: uid.into(),
                iteration: String::new(),
                uri: format!("spotify:track:{uid}"),
            },
            provider: "queue".into(),
            track: Track::placeholder(&format!("spotify:track:{uid}")),
            queued_by: by.map(str::to_owned),
        };
        let rows = [
            row("a", Some("friend")),
            row("b", None),
            row("c", Some("someone")),
            row("d", Some("friend")),
        ];
        let refs: Vec<&super::super::state::QueueRow> = rows.iter().collect();
        assert_eq!(queued_by(&refs), vec!["friend", "someone"]);
    }

    #[test]
    fn describes_jam_writes() {
        assert!(JamAction::Leave("s".into()).ends());
        assert!(JamAction::End("s".into()).ends());
        assert!(!JamAction::Start.ends());
        assert_eq!(JamAction::Start.reason(), "NEW_SESSION");
        assert_eq!(
            JamAction::Join {
                token: "t".into(),
                mode: JoinMode::Remote
            }
            .reason(),
            "YOU_JOINED"
        );
        assert_eq!(
            JamAction::QueueOnly(true).failure(),
            "Couldn’t change the Jam settings. Try again."
        );
    }

    #[test]
    fn locks_only_a_guest_in_a_queue_only_jam() {
        assert!(!queue_only_guest(None));
        let host = Session {
            queue_only: true,
            ..session(true)
        };
        assert!(!queue_only_guest(Some(&host)));
        assert!(!queue_only_guest(Some(&session(false))));
        let guest = Session {
            queue_only: true,
            ..session(false)
        };
        assert!(queue_only_guest(Some(&guest)));
    }

    #[test]
    fn keeps_the_volume_with_the_host_when_a_guest_cannot_reach_it() {
        let speaker = |this_mac| ConnectDevice {
            this_mac,
            ..ConnectDevice::default()
        };
        let (mac, remote) = (speaker(true), speaker(false));
        let guest = Session {
            volume_control: VolumeControl::Enabled,
            ..session(false)
        };
        assert!(!host_volume_only(None, None));
        assert!(!host_volume_only(Some(&guest), None));
        assert!(!host_volume_only(Some(&guest), Some(&remote)));
        let locked = Session {
            volume_control: VolumeControl::Disabled,
            ..guest.clone()
        };
        assert!(host_volume_only(Some(&locked), None));
        assert!(host_volume_only(Some(&locked), Some(&remote)));
        assert!(!host_volume_only(Some(&locked), Some(&mac)));
        let redirected = Session {
            redirect_commands: true,
            ..guest
        };
        assert!(host_volume_only(Some(&redirected), None));
        assert!(!host_volume_only(Some(&redirected), Some(&remote)));
        assert!(!host_volume_only(Some(&redirected), Some(&mac)));
    }

    #[test]
    fn names_people_and_roles() {
        let jam = session(true);
        assert_eq!(people_label(1), "1 person");
        assert_eq!(people_label(3), "3 people");
        assert_eq!(
            faces(&jam)
                .iter()
                .map(|face| face.name.as_str())
                .collect::<Vec<_>>(),
            vec!["Host Person", "Guest Person"]
        );
        assert_eq!(role(&jam, &jam.members[1]), "Host");
        assert_eq!(role(&jam, &jam.members[0]), "Listening along");
        let plain = Member {
            id: "other".into(),
            ..Member::default()
        };
        assert_eq!(role(&jam, &plain), "Guest");
    }
}
