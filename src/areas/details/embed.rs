use std::{rc::Rc, time::Duration};

use gpui::{
    App, ClipboardItem, Context, Div, Entity, Focusable, FontWeight, HighlightStyle,
    InteractiveText, SharedString, StyledText, Subscription, Task, Window, div, prelude::*, px,
    rgb, rgba,
};
use gpui_component::{
    checkbox::Checkbox,
    input::{Input, InputEvent, InputState},
    tooltip::Tooltip,
};
use spotify_client::share::{self, Embed, Kind, Size, Theme, Width};

use super::{dialog, state::Shown, worker::Command};
use crate::{app::OpenMusic, ui::Palette};

const WIDTH: f32 = 520.;
const COPIED_FOR: Duration = Duration::from_secs(5);
const DEVELOPER_TERMS: &str = "https://developer.spotify.com/terms/";
const PLATFORM_RULES: &str = "https://www.spotify.com/safetyandprivacy/platform-rules";
const WIDTH_HELP: &str = "When set to 100%, the player width will automatically expand to fit mobile and desktop layouts.";
const TERMS: &str = "By embedding a Spotify player on your site, you are agreeing to Spotify’s Developer Terms and Spotify Platform Rules";
const TERMS_LINKS: [(&str, &str); 2] = [
    ("Spotify’s Developer Terms", DEVELOPER_TERMS),
    ("Spotify Platform Rules", PLATFORM_RULES),
];

pub struct EmbedView {
    pub uri: String,
    kind: Kind,
    pub theme: Theme,
    pub size: Size,
    pub width: Width,
    pub start_on: bool,
    pub start: u32,
    pub show_code: bool,
    pub swatch: Option<u32>,
    copied: bool,
    reset: Option<Task<()>>,
    share_id: String,
    width_input: Entity<InputState>,
    start_input: Entity<InputState>,
    _subscriptions: [Subscription; 2],
}

impl EmbedView {
    pub fn new(
        uri: String,
        kind: Kind,
        start: u32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let width = Width::default();
        let width_input = cx.new(|cx| InputState::new(window, cx).default_value(width.label()));
        let start_input =
            cx.new(|cx| InputState::new(window, cx).default_value(share::format_time(start)));
        let subscriptions = [
            cx.subscribe_in(&width_input, window, |this, input, event, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                    let text = input.read(cx).value();
                    this.set_width(&text, window, cx);
                }
            }),
            cx.subscribe_in(&start_input, window, |this, input, event, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                    let text = input.read(cx).value();
                    this.set_start(&text, window, cx);
                }
            }),
        ];
        Self {
            uri,
            kind,
            theme: Theme::default(),
            size: Size::default(),
            width,
            start_on: false,
            start,
            show_code: false,
            swatch: None,
            copied: false,
            reset: None,
            share_id: share::share_id(),
            width_input,
            start_input,
            _subscriptions: subscriptions,
        }
    }

    pub fn embed(&self) -> Embed {
        Embed {
            theme: self.theme,
            size: self.size,
            width: self.width,
            start: (self.kind.takes_start() && self.start_on).then_some(self.start),
            share_id: Some(self.share_id.clone()),
        }
    }

    pub fn code(&self) -> String {
        share::embed_code(&self.uri, &self.embed()).unwrap_or_default()
    }

    pub fn set_width(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.width = Width::parse(text);
        let label = self.width.label();
        if *text != label {
            self.width_input
                .update(cx, |input, cx| input.set_value(label, window, cx));
        }
        cx.notify();
    }

    pub fn set_start(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(start) = share::parse_time(text) {
            self.start = start;
        }
        let label = share::format_time(self.start);
        if *text != label {
            self.start_input
                .update(cx, |input, cx| input.set_value(label, window, cx));
        }
        cx.notify();
    }

    pub fn typing(&self, window: &Window, cx: &App) -> bool {
        [&self.width_input, &self.start_input]
            .into_iter()
            .any(|input| input.read(cx).focus_handle(cx).is_focused(window))
    }

    pub fn copy(&mut self, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(self.code()));
        self.copied = true;
        self.reset = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(COPIED_FOR).await;
            let _ = this.update(cx, |this, cx| {
                this.copied = false;
                cx.notify();
            });
        }));
        cx.notify();
    }

    pub fn describe(&self) -> String {
        [
            format!("uri={}", self.uri),
            format!("title={}", self.kind.embed_title().unwrap_or_default()),
            format!("theme={:?} swatch={:?}", self.theme, self.swatch),
            format!("size={} width={}", self.size.label(), self.width.label()),
            format!(
                "start={} on={} offered={}",
                share::format_time(self.start),
                self.start_on,
                self.kind.takes_start()
            ),
            format!("show_code={} copied={}", self.show_code, self.copied),
            format!(
                "url={}",
                share::embed_url(&self.uri, &self.embed()).unwrap_or_default()
            ),
            format!("code={}", self.code()),
        ]
        .join("\n")
    }

    fn label(text: &'static str, palette: Palette) -> Div {
        div()
            .w(px(52.))
            .flex_shrink_0()
            .text_size(px(13.))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(rgb(palette.text))
            .child(text)
    }

    fn colors(&self, palette: Palette, cx: &mut Context<Self>) -> Div {
        let swatches = [
            (Theme::Dynamic, self.swatch.unwrap_or(0xffffff)),
            (Theme::Dark, Theme::DARK_SWATCH),
        ];
        div()
            .flex()
            .items_center()
            .gap_2()
            .child(Self::label("Color", palette))
            .children(swatches.into_iter().map(|(theme, color)| {
                let selected = self.theme == theme;
                div()
                    .id(match theme {
                        Theme::Dynamic => "embed-color-dynamic",
                        Theme::Dark => "embed-color-dark",
                    })
                    .size(px(28.))
                    .p(px(2.))
                    .rounded_full()
                    .cursor_pointer()
                    .border_2()
                    .border_color(if selected {
                        rgb(palette.accent)
                    } else {
                        rgba(0)
                    })
                    .child(
                        div()
                            .size_full()
                            .rounded_full()
                            .bg(rgb(color))
                            .border_1()
                            .border_color(rgba(dialog::outline(palette))),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.theme = theme;
                        cx.notify();
                    }))
            }))
    }

    fn sizes(&self, palette: Palette, cx: &mut Context<Self>) -> Div {
        let segments = div()
            .flex()
            .p(px(2.))
            .rounded_full()
            .bg(rgba(palette.press))
            .children([Size::Normal, Size::Compact].into_iter().map(|size| {
                let selected = self.size == size;
                div()
                    .id(match size {
                        Size::Normal => "embed-size-normal",
                        Size::Compact => "embed-size-compact",
                    })
                    .h(px(24.))
                    .px_3()
                    .flex()
                    .items_center()
                    .rounded_full()
                    .cursor_pointer()
                    .text_size(px(12.5))
                    .font_weight(FontWeight::MEDIUM)
                    .map(|segment| {
                        if selected {
                            segment
                                .bg(rgb(if palette.dark { 0x5a5960 } else { 0xffffff }))
                                .shadow_sm()
                                .text_color(rgb(palette.text))
                        } else {
                            segment
                                .text_color(rgb(palette.muted))
                                .hover(move |segment| segment.text_color(rgb(palette.text)))
                        }
                    })
                    .child(size.label())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.size = size;
                        cx.notify();
                    }))
            }));
        div()
            .flex()
            .items_center()
            .gap_2()
            .child(Self::label("Size", palette))
            .child(segments)
            .child(div().text_color(rgb(palette.muted)).child("×"))
            .child(field(&self.width_input, 72., palette))
            .child(
                div()
                    .id("embed-width-help")
                    .size(px(18.))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .border_1()
                    .border_color(rgba(dialog::outline(palette)))
                    .text_size(px(11.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(palette.muted))
                    .child("?")
                    .tooltip(|window, cx| Tooltip::new(WIDTH_HELP).build(window, cx)),
            )
    }

    fn terms(palette: Palette) -> impl IntoElement {
        let link = HighlightStyle {
            color: Some(rgb(palette.accent).into()),
            ..HighlightStyle::default()
        };
        let ranges: Vec<std::ops::Range<usize>> = TERMS_LINKS
            .iter()
            .filter_map(|(text, _)| TERMS.find(text).map(|start| start..start + text.len()))
            .collect();
        let text = StyledText::new(TERMS)
            .with_highlights(ranges.iter().map(|range| (range.clone(), link)));
        div()
            .text_size(px(12.))
            .line_height(px(17.))
            .text_color(rgb(palette.muted))
            .child(
                InteractiveText::new("embed-terms", text).on_click(ranges, |index, _, cx| {
                    if let Some((_, url)) = TERMS_LINKS.get(index) {
                        dialog::open_url(url, cx);
                    }
                }),
            )
    }

    fn actions(&self, palette: Palette, cx: &mut Context<Self>) -> Div {
        let this = cx.entity().downgrade();
        let start = self.kind.takes_start().then(|| {
            let this = this.clone();
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    Checkbox::new("embed-start")
                        .label("Start at")
                        .checked(self.start_on)
                        .on_click(move |checked, _, cx| {
                            let checked = *checked;
                            let _ = this.update(cx, |view, cx| {
                                view.start_on = checked;
                                cx.notify();
                            });
                        }),
                )
                .child(field(&self.start_input, 64., palette))
        });
        div()
            .flex()
            .items_center()
            .gap_3()
            .children(start)
            .child(div().flex_1())
            .child(
                Checkbox::new("embed-show-code")
                    .label("Show code")
                    .checked(self.show_code)
                    .on_click(move |checked, _, cx| {
                        let checked = *checked;
                        let _ = this.update(cx, |view, cx| {
                            view.show_code = checked;
                            cx.notify();
                        });
                    }),
            )
            .child(
                dialog::pill(
                    "embed-copy",
                    if self.copied { "Copied!" } else { "Copy" },
                    true,
                    palette,
                )
                .on_click(cx.listener(|this, _, _, cx| this.copy(cx))),
            )
    }
}

fn field(input: &Entity<InputState>, width: f32, palette: Palette) -> Div {
    div()
        .w(px(width))
        .h(px(28.))
        .flex_shrink_0()
        .flex()
        .items_center()
        .px_2()
        .rounded_full()
        .bg(rgba(palette.press))
        .text_size(px(13.))
        .child(
            Input::new(input)
                .appearance(false)
                .bordered(false)
                .focus_bordered(false),
        )
}

impl Render for EmbedView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = Palette::for_window(window.appearance());
        div()
            .id("embed-scroll")
            .w_full()
            .max_h(px(dialog::body_height(window)))
            .overflow_y_scroll()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .child(self.colors(palette, cx))
                    .child(self.sizes(palette, cx))
                    .child(Self::terms(palette))
                    .child(self.actions(palette, cx))
                    .when(self.show_code, |body| {
                        body.child(
                            div()
                                .w_full()
                                .overflow_hidden()
                                .p_3()
                                .rounded(px(10.))
                                .bg(rgba(palette.press))
                                .font_family("Menlo")
                                .text_size(px(11.5))
                                .line_height(px(16.))
                                .text_color(rgb(palette.text))
                                .child(SharedString::from(self.code())),
                        )
                    }),
            )
    }
}

impl OpenMusic {
    pub fn show_embed(&mut self, uri: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(kind) = Kind::of(&uri) else {
            return;
        };
        let Some(title) = kind.embed_title() else {
            return;
        };
        let start = if self.is_current(&uri) {
            self.playback.now.position() / 1000
        } else {
            0
        };
        let view = cx.new(|cx| EmbedView::new(uri.clone(), kind, start, window, cx));
        self.send(Command::CoverColor(uri.clone()));
        let close = dialog::redraw(cx.weak_entity());
        let shown = Shown::Embed(uri, view.clone());
        let build: dialog::Build = Rc::new(move |dialog, window, _| {
            dialog::frame(dialog, window, WIDTH, title, None, close.clone()).child(view.clone())
        });
        self.details.show(shown, build, window, cx);
    }
}
