use std::{
    collections::HashMap,
    rc::Rc,
    time::{Duration, Instant},
};

use gpui::{
    AnyElement, Context, Div, ElementId, FontWeight, SharedString, WeakEntity, Window, div,
    prelude::*, px, relative, rgb, rgba,
};
use gpui_component::WindowExt;
use spotify_client::details::{Contributor, Credits};

use super::{dialog, worker::Command};
use crate::{
    app::{OpenMusic, Page},
    ui::{Palette, components::bone},
};

pub const REPORT_URL: &str = "https://support.spotify.com/artists/article/song-credits/";
const WIDTH: f32 = 480.;
const KEEP_FOR: Duration = Duration::from_secs(600);
const KEEP_AT_MOST: usize = 20;

struct Cached {
    at: Instant,
    order: u64,
    credits: Credits,
}

#[derive(Default)]
pub struct Cache {
    entries: HashMap<String, Cached>,
    next: u64,
}

impl Cache {
    pub fn get(&self, uri: &str) -> Option<Credits> {
        self.entries
            .get(uri)
            .filter(|cached| cached.at.elapsed() < KEEP_FOR)
            .map(|cached| cached.credits.clone())
    }

    pub fn insert(&mut self, credits: Credits) {
        self.entries
            .retain(|uri, cached| cached.at.elapsed() < KEEP_FOR && *uri != credits.uri);
        if self.entries.len() >= KEEP_AT_MOST
            && let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, cached)| cached.order)
                .map(|(uri, _)| uri.clone())
        {
            self.entries.remove(&oldest);
        }
        self.next += 1;
        self.entries.insert(
            credits.uri.clone(),
            Cached {
                at: Instant::now(),
                order: self.next,
                credits,
            },
        );
    }
}

pub fn roles(roles: &[String]) -> String {
    roles
        .iter()
        .map(|role| capitalize(role))
        .collect::<Vec<_>>()
        .join(" • ")
}

fn capitalize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut start = true;
    for character in text.chars() {
        if start {
            out.extend(character.to_uppercase());
        } else {
            out.push(character);
        }
        start = character.is_whitespace() || character == '-';
    }
    out
}

pub struct CreditsView {
    app: WeakEntity<OpenMusic>,
    pub uri: String,
    title: String,
    pub result: Option<Result<Credits, String>>,
}

impl CreditsView {
    pub fn subtitle(&self) -> String {
        match &self.result {
            Some(Ok(credits)) if !credits.track.is_empty() => credits.track.clone(),
            _ => self.title.clone(),
        }
    }

    pub fn describe(&self) -> String {
        let state = match &self.result {
            None => "loading",
            Some(Ok(_)) => "ready",
            Some(Err(_)) => "error",
        };
        let mut lines = vec![
            format!("uri={} state={state}", self.uri),
            format!("title={}", self.subtitle()),
        ];
        match &self.result {
            Some(Ok(credits)) => {
                for group in &credits.groups {
                    lines.push(format!("# {}", group.name));
                    for person in &group.people {
                        let mut line = format!("{} — {}", person.name, roles(&person.roles));
                        if let Some(link) = person.uri.as_ref().or(person.url.as_ref()) {
                            line.push_str(&format!(" [{link}]"));
                        }
                        lines.push(line);
                    }
                }
                lines.push(format!("sources={}", credits.sources.join(", ")));
            }
            Some(Err(message)) => lines.push(format!("error={message}")),
            None => {}
        }
        lines.join("\n")
    }

    fn close_and(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
        then: impl FnOnce(&mut OpenMusic) + 'static,
    ) {
        window.close_dialog(cx);
        let _ = self.app.update(cx, |app, cx| {
            app.details.credits_view = None;
            then(app);
            cx.notify();
        });
    }

    fn person(
        &self,
        id: ElementId,
        person: &Contributor,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let following = person.uri.as_ref().and_then(|uri| {
            self.app
                .upgrade()
                .and_then(|app| app.read(cx).is_saved(uri))
        });
        let text = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .child(
                div()
                    .truncate()
                    .text_size(px(14.))
                    .line_height(px(20.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(rgb(palette.text))
                    .child(person.name.clone()),
            )
            .when(!person.roles.is_empty(), |text| {
                text.child(
                    div()
                        .text_size(px(12.5))
                        .line_height(px(17.))
                        .text_color(rgb(palette.muted))
                        .child(roles(&person.roles)),
                )
            });
        let row = div()
            .id(id.clone())
            .flex()
            .items_center()
            .gap_3()
            .px_2()
            .py(px(6.))
            .rounded(px(8.))
            .child(text);
        if let Some(uri) = person.uri.clone() {
            let artist = uri.clone();
            return row
                .cursor_pointer()
                .hover(move |row| row.bg(rgba(palette.press)))
                .children(following.map(|following| {
                    let follow = uri.clone();
                    dialog::pill(
                        ElementId::NamedChild(Box::new(id.clone()), "follow".into()),
                        if following { "Following" } else { "Follow" },
                        false,
                        palette,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        let uris = vec![follow.clone()];
                        let _ = this.app.update(cx, |app, cx| {
                            app.set_saved(uris, !following);
                            cx.notify();
                        });
                        cx.notify();
                    }))
                }))
                .on_click(cx.listener(move |this, _, window, cx| {
                    let page = Page::Artist(artist.clone());
                    this.close_and(window, cx, move |app| app.navigate(page));
                }))
                .into_any_element();
        }
        if let Some(url) = person.url.clone() {
            return row
                .cursor_pointer()
                .hover(move |row| row.bg(rgba(palette.press)))
                .child(
                    div()
                        .flex_shrink_0()
                        .text_size(px(14.))
                        .text_color(rgb(palette.muted))
                        .child("↗"),
                )
                .on_click(move |_, _, cx| dialog::open_url(&url, cx))
                .into_any_element();
        }
        row.into_any_element()
    }

    fn list(&self, credits: &Credits, palette: Palette, cx: &mut Context<Self>) -> Div {
        if credits.groups.is_empty() && credits.sources.is_empty() {
            return message(spotify_client::details::Unavailable.to_string(), palette);
        }
        let mut body = div().flex().flex_col().gap_5();
        for (group_index, group) in credits.groups.iter().enumerate() {
            let people: Vec<AnyElement> = group
                .people
                .iter()
                .enumerate()
                .map(|(index, person)| {
                    let id = ElementId::Name(format!("credit-{group_index}-{index}").into());
                    self.person(id, person, palette, cx)
                })
                .collect();
            body = body.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_0p5()
                    .child(
                        dialog::section_title(group.name.clone(), palette)
                            .px_2()
                            .pb_1(),
                    )
                    .children(people),
            );
        }
        if !credits.sources.is_empty() {
            body = body.child(
                div()
                    .px_2()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(dialog::section_title("Sources", palette))
                    .child(
                        div()
                            .text_size(px(13.))
                            .line_height(px(18.))
                            .text_color(rgb(palette.muted))
                            .child(credits.sources.join(", ")),
                    ),
            );
        }
        body.child(
            div().px_2().flex().child(
                dialog::pill("credits-report", "Report error ↗", false, palette)
                    .on_click(|_, _, cx| dialog::open_url(REPORT_URL, cx)),
            ),
        )
    }
}

fn message(text: impl Into<SharedString>, palette: Palette) -> Div {
    div()
        .px_2()
        .py_4()
        .text_size(px(14.))
        .text_color(rgb(palette.muted))
        .child(text.into())
}

fn loading(palette: Palette) -> Div {
    let widths = [0.34, 0.46, 0.4, 0.3, 0.52];
    div()
        .px_2()
        .flex()
        .flex_col()
        .gap_4()
        .child(
            bone("credits", 0, palette)
                .w(px(72.))
                .h(px(15.))
                .rounded(px(4.)),
        )
        .children(widths.iter().enumerate().map(|(index, width)| {
            div()
                .flex()
                .flex_col()
                .gap_1p5()
                .child(
                    bone("credits", index * 2 + 1, palette)
                        .w(relative(*width))
                        .h(px(13.))
                        .rounded(px(4.)),
                )
                .child(
                    bone("credits", index * 2 + 2, palette)
                        .w(relative(width * 0.6))
                        .h(px(11.))
                        .rounded(px(4.)),
                )
        }))
}

impl Render for CreditsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = Palette::for_window(window.appearance());
        let body = match &self.result {
            None => loading(palette),
            Some(Err(text)) => message(text.clone(), palette),
            Some(Ok(credits)) => {
                let credits = credits.clone();
                self.list(&credits, palette, cx)
            }
        };
        div()
            .id("credits-scroll")
            .w_full()
            .max_h(px(dialog::body_height(window)))
            .overflow_y_scroll()
            .child(body)
    }
}

impl OpenMusic {
    pub fn show_credits(
        &mut self,
        uri: String,
        title: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let cached = self.details.credits.get(&uri);
        match &cached {
            Some(credits) => self.lookup_saved(credits.linked_artists()),
            None => self.send(Command::Credits(uri.clone())),
        }
        let app = cx.weak_entity();
        let view = cx.new(|_| CreditsView {
            app: app.clone(),
            uri: uri.clone(),
            title,
            result: cached.map(Ok),
        });
        self.details.close_dialogs(window, cx);
        self.details.credits_view = Some((uri, view.clone()));
        let close: dialog::Close = Rc::new(move |_, cx| {
            let _ = app.update(cx, |app, cx| {
                app.details.credits_view = None;
                cx.notify();
            });
        });
        window.open_dialog(cx, move |dialog, window, cx| {
            let subtitle = view.read(cx).subtitle();
            dialog::frame(
                dialog,
                window,
                WIDTH,
                "Credits",
                (!subtitle.is_empty()).then(|| subtitle.into()),
                close.clone(),
            )
            .child(view.clone())
        });
    }
}

#[cfg(test)]
mod tests {
    use spotify_client::details::CreditGroup;

    use super::*;

    fn credits(uri: &str) -> Credits {
        Credits {
            uri: uri.into(),
            track: "Creep".into(),
            groups: vec![CreditGroup {
                name: "Artist".into(),
                people: Vec::new(),
            }],
            sources: Vec::new(),
        }
    }

    #[test]
    fn joins_and_capitalizes_roles() {
        assert_eq!(
            roles(&[
                "Main Artist".into(),
                "bass guitar".into(),
                "co-producer".into()
            ]),
            "Main Artist • Bass Guitar • Co-Producer"
        );
        assert_eq!(roles(&[]), "");
    }

    #[test]
    fn keeps_the_twenty_newest_credits() {
        let mut cache = Cache::default();
        for index in 0..=KEEP_AT_MOST {
            cache.insert(credits(&format!("spotify:track:{index:022}")));
        }
        assert_eq!(cache.entries.len(), KEEP_AT_MOST);
        cache.insert(credits(&format!("spotify:track:{KEEP_AT_MOST:022}")));
        assert_eq!(cache.entries.len(), KEEP_AT_MOST);
        assert!(cache.get(&format!("spotify:track:{:022}", 1)).is_some());
        assert!(cache.get(&format!("spotify:track:{:022}", 0)).is_none());
        let newest = format!("spotify:track:{KEEP_AT_MOST:022}");
        assert_eq!(cache.get(&newest).map(|credits| credits.uri), Some(newest));
    }
}
