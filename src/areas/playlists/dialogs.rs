use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use gpui::{
    AnyElement, App, Context, Entity, FontWeight, Hsla, Image, ImageFormat, ObjectFit,
    PathPromptOptions, SharedString, Subscription, WeakEntity, Window, div, hsla, img, prelude::*,
    px, svg,
};
use gpui_component::{
    ActiveTheme, Sizable, WindowExt,
    button::{Button, ButtonVariants},
    dialog::DialogButtonProps,
    input::{Input, InputEvent, InputState, SelectAll},
};
use spotify_client::playlist::{Details, EditTarget, Level, Member, TargetKind, editable_tree};

use super::{image, worker::Cover};
use crate::{app::OpenMusic, icons, worker::trusted_image};

pub struct Draft {
    pub uri: String,
    pub name: String,
    pub description: String,
    pub cover: Option<String>,
    pub custom: bool,
}

enum Photo {
    Keep,
    New { jpeg: Vec<u8>, preview: Arc<Image> },
    Remove,
}

pub struct EditDetails {
    app: WeakEntity<OpenMusic>,
    uri: String,
    name: Entity<InputState>,
    description: Entity<InputState>,
    original_name: String,
    original_description: String,
    cover: Option<String>,
    custom: bool,
    photo: Photo,
    error: Option<String>,
    preparing: bool,
}

impl EditDetails {
    pub fn new(
        app: WeakEntity<OpenMusic>,
        draft: Draft,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let name = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Add a name")
                .default_value(draft.name.clone())
        });
        let description = cx.new(|cx| {
            InputState::new(window, cx)
                .multi_line(true)
                .rows(4)
                .placeholder("Add an optional description")
                .default_value(draft.description.clone())
        });
        Self {
            app,
            uri: draft.uri,
            name,
            description,
            original_name: draft.name,
            original_description: draft.description,
            cover: draft.cover,
            custom: draft.custom,
            photo: Photo::Keep,
            error: None,
            preparing: false,
        }
    }

    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.name.update(cx, |input, cx| input.focus(window, cx));
    }

    fn choose_photo(&mut self, cx: &mut Context<Self>) {
        if self.preparing {
            return;
        }
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Choose photo".into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = paths.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let started = this.update(cx, |edit, cx| {
                edit.preparing = true;
                edit.error = None;
                cx.notify();
            });
            if started.is_err() {
                return;
            }
            let result = cx
                .background_executor()
                .spawn(async move { image::cover_jpeg(&path) })
                .await;
            let _ = this.update(cx, |edit, cx| {
                edit.preparing = false;
                match result {
                    Ok(jpeg) => {
                        edit.photo = Photo::New {
                            preview: Arc::new(Image::from_bytes(ImageFormat::Jpeg, jpeg.clone())),
                            jpeg,
                        }
                    }
                    Err(message) => edit.error = Some(message),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn remove_photo(&mut self, cx: &mut Context<Self>) {
        self.photo = if self.custom {
            Photo::Remove
        } else {
            Photo::Keep
        };
        self.error = None;
        cx.notify();
    }

    pub fn save(&mut self, cx: &mut Context<Self>) -> bool {
        if self.preparing {
            return false;
        }
        let name = self.name.read(cx).value().trim().to_owned();
        let description = self.description.read(cx).value().trim().to_owned();
        if name.is_empty() {
            self.error = Some("Playlist name is required.".into());
            cx.notify();
            return false;
        }
        let details = Details {
            name: (name != self.original_name).then_some(name),
            description: (description != self.original_description)
                .then(|| (!description.is_empty()).then_some(description)),
            picture: None,
        };
        let cover = match &self.photo {
            Photo::Keep => Cover::Keep,
            Photo::New { jpeg, .. } => Cover::Set(jpeg.clone()),
            Photo::Remove => Cover::Remove,
        };
        if details.is_empty() && matches!(cover, Cover::Keep) {
            return true;
        }
        if let Err(error) = details.changes() {
            self.error = Some(error.to_string());
            cx.notify();
            return false;
        }
        let uri = self.uri.clone();
        self.app
            .update(cx, |app, cx| app.save_details(uri, details, cover, cx))
            .is_ok()
    }
}

fn label(text: &'static str, color: Hsla) -> impl IntoElement {
    div()
        .text_size(px(12.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(color)
        .child(text)
}

fn action(
    id: &'static str,
    text: &'static str,
    color: Hsla,
    run: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    div()
        .id(id)
        .text_size(px(12.5))
        .font_weight(FontWeight::MEDIUM)
        .text_color(color)
        .cursor_pointer()
        .hover(|text| text.underline())
        .on_click(run)
        .child(text)
}

impl Render for EditDetails {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (text, muted, danger, tile, primary) = (
            theme.foreground,
            theme.muted_foreground,
            theme.danger,
            theme.muted,
            theme.primary,
        );
        let current = match &self.photo {
            Photo::New { preview, .. } => Some(preview.clone()),
            Photo::Remove => None,
            Photo::Keep => self.cover.as_ref().and_then(|url| {
                self.app
                    .upgrade()
                    .and_then(|app| app.read(cx).covers.get(url).cloned())
            }),
        };
        let removable = match &self.photo {
            Photo::New { .. } => true,
            Photo::Remove => false,
            Photo::Keep => self.custom,
        };
        let chosen = current.is_some();
        let preparing = self.preparing;
        let cover = div()
            .id("edit-cover")
            .group("edit-cover")
            .relative()
            .size(px(180.))
            .flex_shrink_0()
            .rounded(px(8.))
            .overflow_hidden()
            .bg(tile)
            .cursor_pointer()
            .flex()
            .items_center()
            .justify_center()
            .map(|tile| match current {
                Some(image) => tile.child(img(image).size_full().object_fit(ObjectFit::Cover)),
                None => tile.child(svg().path(icons::NOTE).size(px(60.)).text_color(muted)),
            })
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(hsla(0., 0., 0., 0.55))
                    .text_color(hsla(0., 0., 1., 1.))
                    .text_size(px(13.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .when(!preparing, |overlay| {
                        overlay
                            .opacity(0.)
                            .group_hover("edit-cover", |style| style.opacity(1.))
                    })
                    .child(if preparing {
                        "Preparing image…"
                    } else {
                        "Choose photo"
                    }),
            )
            .on_click(cx.listener(|this, _, _, cx| this.choose_photo(cx)));
        div()
            .flex()
            .flex_col()
            .gap_3()
            .text_color(text)
            .child(
                div()
                    .flex()
                    .gap_4()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(cover)
                            .child(
                                div()
                                    .flex()
                                    .gap_3()
                                    .child(action(
                                        "edit-change-photo",
                                        if chosen { "Change photo" } else { "Choose photo" },
                                        primary,
                                        cx.listener(|this, _, _, cx| this.choose_photo(cx)),
                                    ))
                                    .when(removable, |links| {
                                        links.child(action(
                                            "edit-remove-photo",
                                            "Remove photo",
                                            primary,
                                            cx.listener(|this, _, _, cx| this.remove_photo(cx)),
                                        ))
                                    }),
                            ),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap_1p5()
                            .child(label("Name", muted))
                            .child(Input::new(&self.name))
                            .child(div().mt_2().child(label("Description", muted)))
                            .child(Input::new(&self.description).h(px(108.))),
                    ),
            )
            .when_some(self.error.clone(), |body, error| {
                body.child(div().text_size(px(12.5)).text_color(danger).child(error))
            })
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(muted)
                    .child("By proceeding, you agree to give Spotify access to the image you choose to upload. Please make sure you have the right to upload the image."),
            )
    }
}

pub fn open_edit_details(window: &mut Window, cx: &mut App, edit: Entity<EditDetails>) {
    window.open_dialog(cx, move |dialog, _, cx| {
        let save = edit.clone();
        dialog
            .title("Edit details")
            .w(px(560.))
            .bg(cx.theme().popover)
            .child(edit.clone())
            .button_props(DialogButtonProps::default().ok_text("Save"))
            .footer(|ok, _, window, cx| vec![ok(window, cx)])
            .on_ok(move |_, _, cx| save.update(cx, |edit, cx| edit.save(cx)))
    });
}

pub struct Collaborators {
    app: WeakEntity<OpenMusic>,
    uri: String,
}

impl Collaborators {
    pub fn new(app: WeakEntity<OpenMusic>, uri: String) -> Self {
        Self { app, uri }
    }

    fn with_app(
        &self,
        cx: &mut Context<Self>,
        run: impl FnOnce(&mut OpenMusic, &mut Context<OpenMusic>),
    ) {
        let _ = self.app.update(cx, |app, cx| {
            run(app, cx);
            cx.notify();
        });
    }
}

pub fn role(member: &Member) -> &'static str {
    if member.owner {
        "Creator"
    } else if member.level == Some(Level::Contributor) {
        "Collaborator"
    } else {
        "Listener"
    }
}

fn initial(name: &str) -> String {
    name.chars()
        .find(|letter| letter.is_alphanumeric())
        .map(|letter| letter.to_uppercase().collect())
        .unwrap_or_else(|| "?".to_owned())
}

fn face(name: &str, image: Option<Arc<Image>>, fill: Hsla, text: Hsla) -> impl IntoElement {
    let circle = div()
        .size(px(36.))
        .flex_shrink_0()
        .rounded_full()
        .overflow_hidden()
        .flex()
        .items_center()
        .justify_center();
    match image {
        Some(image) => circle.child(img(image).size_full().object_fit(ObjectFit::Cover)),
        None => circle
            .bg(fill)
            .text_color(text)
            .text_size(px(15.))
            .font_weight(FontWeight::SEMIBOLD)
            .child(initial(name)),
    }
}

impl Render for Collaborators {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (text, muted, fill) = (theme.foreground, theme.muted_foreground, theme.muted);
        let body = div().flex().flex_col().gap_3().text_color(text);
        let Some(app) = self.app.upgrade() else {
            return body;
        };
        let app = app.read(cx);
        let me = app.playlists.user.clone();
        let Some(permissions) = app.playlists.permissions.get(&self.uri).cloned() else {
            return body.child(
                div()
                    .text_size(px(13.))
                    .text_color(muted)
                    .child("Loading collaborators…"),
            );
        };
        let images: Vec<Option<Arc<Image>>> = permissions
            .members
            .iter()
            .map(|member| {
                member
                    .image
                    .as_deref()
                    .filter(|url| trusted_image(url))
                    .and_then(|url| app.covers.get(url).cloned())
            })
            .collect();
        let visibility = if permissions.base == Some(Level::Blocked) {
            "Private Playlist"
        } else {
            "Public Playlist"
        };
        let uri = self.uri.clone();
        body.child(
            div()
                .text_size(px(12.))
                .text_color(muted)
                .child(format!("{visibility} • Shared with")),
        )
        .children(permissions.members.iter().zip(images).enumerate().map(
            |(index, (member, image))| {
                let mine = me.as_deref() == Some(member.username.as_str());
                let name: SharedString = if mine {
                    format!("{} (you)", member.name).into()
                } else {
                    member.name.clone().into()
                };
                let manage = permissions.administrate && !member.owner && !mine;
                let leave = mine && !member.owner && permissions.cancel_membership;
                let contributor = member.level == Some(Level::Contributor);
                let (toggle_uri, remove_uri, leave_uri) = (uri.clone(), uri.clone(), uri.clone());
                let (toggle_user, remove_user) = (member.username.clone(), member.username.clone());
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(face(&member.name, image, fill, text))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .text_size(px(13.5))
                                    .font_weight(FontWeight::MEDIUM)
                                    .truncate()
                                    .child(name),
                            )
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(muted)
                                    .child(role(member)),
                            ),
                    )
                    .when(manage, |row| {
                        row.child(
                            Button::new(("collaborator-role", index))
                                .small()
                                .ghost()
                                .label(if contributor {
                                    "Remove as collaborator"
                                } else {
                                    "Make collaborator"
                                })
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    let level = if contributor {
                                        Level::Viewer
                                    } else {
                                        Level::Contributor
                                    };
                                    let (uri, user) = (toggle_uri.clone(), toggle_user.clone());
                                    this.with_app(cx, move |app, _| {
                                        app.set_playlist_member(uri, user, level)
                                    });
                                })),
                        )
                        .child(
                            Button::new(("collaborator-remove", index))
                                .small()
                                .ghost()
                                .label("Remove from playlist")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    let (uri, user) = (remove_uri.clone(), remove_user.clone());
                                    this.with_app(cx, move |app, _| {
                                        app.remove_playlist_member(uri, user)
                                    });
                                })),
                        )
                    })
                    .when(leave, |row| {
                        row.child(
                            Button::new(("collaborator-leave", index))
                                .small()
                                .ghost()
                                .label("Leave playlist")
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    let uri = leave_uri.clone();
                                    let _ = this
                                        .app
                                        .update(cx, |app, cx| app.leave_playlist(uri, window, cx));
                                })),
                        )
                    })
            },
        ))
        .when(permissions.administrate, |body| {
            let uri = uri.clone();
            body.child(
                div().pt_1().child(
                    Button::new("collaborators-invite")
                        .small()
                        .primary()
                        .label("Invite collaborators")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let uri = uri.clone();
                            this.with_app(cx, move |app, _| app.invite_collaborators(uri));
                        })),
                ),
            )
        })
    }
}

pub fn open_collaborators(
    window: &mut Window,
    cx: &mut App,
    title: SharedString,
    view: Entity<Collaborators>,
) {
    window.open_dialog(cx, move |dialog, _, cx| {
        dialog
            .title(title.clone())
            .w(px(520.))
            .bg(cx.theme().popover)
            .child(view.clone())
    });
}

pub struct FolderName {
    app: WeakEntity<OpenMusic>,
    uri: String,
    name: Entity<InputState>,
    original: String,
    error: Option<String>,
}

impl FolderName {
    pub fn new(
        app: WeakEntity<OpenMusic>,
        uri: String,
        original: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let name = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Folder name")
                .default_value(original.clone())
        });
        Self {
            app,
            uri,
            name,
            original,
            error: None,
        }
    }

    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.name.update(cx, |input, cx| input.focus(window, cx));
        window.dispatch_action(Box::new(SelectAll), cx);
    }

    pub fn save(&mut self, cx: &mut Context<Self>) -> bool {
        let name = self.name.read(cx).value().trim().to_owned();
        if name.is_empty() {
            self.error = Some("Folder name is required".into());
            cx.notify();
            return false;
        }
        if name == self.original {
            return true;
        }
        let uri = self.uri.clone();
        self.app
            .update(cx, |app, _| app.save_folder_name(uri, name))
            .is_ok()
    }
}

impl Render for FolderName {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (text, muted, danger) = (theme.foreground, theme.muted_foreground, theme.danger);
        div()
            .flex()
            .flex_col()
            .gap_1p5()
            .text_color(text)
            .child(label("Name", muted))
            .child(Input::new(&self.name))
            .when_some(self.error.clone(), |body, error| {
                body.child(div().text_size(px(12.5)).text_color(danger).child(error))
            })
    }
}

pub fn open_folder_name(window: &mut Window, cx: &mut App, view: Entity<FolderName>) {
    window.open_dialog(cx, move |dialog, _, cx| {
        let save = view.clone();
        dialog
            .title("Rename")
            .w(px(420.))
            .bg(cx.theme().popover)
            .child(view.clone())
            .button_props(DialogButtonProps::default().ok_text("Save"))
            .footer(|ok, _, window, cx| vec![ok(window, cx)])
            .on_ok(move |_, _, cx| save.update(cx, |view, cx| view.save(cx)))
    });
}

pub struct SaveTo {
    app: WeakEntity<OpenMusic>,
    uris: Vec<String>,
    name: Option<String>,
    search: Entity<InputState>,
    chosen: HashMap<String, bool>,
    _watch: Vec<Subscription>,
}

impl SaveTo {
    pub fn new(
        app: WeakEntity<OpenMusic>,
        uris: Vec<String>,
        name: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Find a playlist"));
        let mut watch = vec![cx.subscribe(&search, |_, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        })];
        if let Some(app) = app.upgrade() {
            watch.push(cx.observe(&app, |_, _, cx| cx.notify()));
        }
        Self {
            app,
            uris,
            name,
            search,
            chosen: HashMap::new(),
            _watch: watch,
        }
    }

    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.search.update(cx, |input, cx| input.focus(window, cx));
    }

    fn targets(&self, cx: &App) -> Option<Result<Vec<EditTarget>, String>> {
        let app = self.app.upgrade()?;
        app.read(cx).playlists.editable_for(&self.uris).cloned()
    }

    fn selected(&self, target: &EditTarget) -> bool {
        self.chosen
            .get(&target.uri)
            .copied()
            .unwrap_or(target.curates)
    }

    pub fn toggle(&mut self, uri: &str, cx: &mut Context<Self>) {
        let Some(Ok(targets)) = self.targets(cx) else {
            return;
        };
        let Some(target) = targets
            .iter()
            .find(|target| target.uri == uri && target.kind != TargetKind::Folder)
        else {
            return;
        };
        let next = !self.selected(target);
        if next == target.curates {
            self.chosen.remove(uri);
        } else {
            self.chosen.insert(uri.to_owned(), next);
        }
        cx.notify();
    }

    pub fn changes(&self, cx: &App) -> Vec<(String, bool)> {
        match self.targets(cx) {
            Some(Ok(targets)) => curation_changes(&targets, &self.chosen),
            _ => Vec::new(),
        }
    }

    pub fn done(&mut self, cx: &mut Context<Self>) -> bool {
        let changes = self.changes(cx);
        if changes.is_empty() {
            return true;
        }
        let items = self.uris.clone();
        self.app
            .update(cx, |app, cx| {
                app.apply_curations(items, changes);
                cx.notify();
            })
            .is_ok()
    }

    fn new_playlist(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.done(cx);
        let (sources, name) = (self.uris.clone(), self.name.clone());
        let _ = self.app.update(cx, |app, cx| {
            app.create_playlist_from(sources, name, None);
            cx.notify();
        });
        window.close_dialog(cx);
    }

    pub fn describe(&self, cx: &App) -> String {
        let mut lines = vec![format!(
            "items={} name={:?} search={:?}",
            self.uris.join(","),
            self.name,
            self.search.read(cx).value().to_string()
        )];
        match self.targets(cx) {
            None => lines.push("loading".into()),
            Some(Err(error)) => lines.push(format!("error {error}")),
            Some(Ok(targets)) => {
                for (depth, target) in editable_tree(&targets) {
                    let mark = match target.kind {
                        TargetKind::Folder => "folder".to_owned(),
                        _ => format!(
                            "{} was={}",
                            if self.selected(target) { "[x]" } else { "[ ]" },
                            u8::from(target.curates)
                        ),
                    };
                    lines.push(format!(
                        "{}{} {} {:?}",
                        "  ".repeat(depth),
                        mark,
                        target.uri,
                        target.name
                    ));
                }
            }
        }
        lines.push(format!("changes={:?}", self.changes(cx)));
        lines.join("\n")
    }

    fn row(
        &self,
        index: usize,
        depth: usize,
        target: &EditTarget,
        picture: Option<Arc<Image>>,
        colors: Colors,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let folder = target.kind == TargetKind::Folder;
        let checked = !folder && self.selected(target);
        let uri = target.uri.clone();
        let fallback = match target.kind {
            TargetKind::Folder => icons::FOLDER,
            TargetKind::LikedSongs => icons::HEART_FILL,
            _ => icons::NOTE,
        };
        let tile = div()
            .size(px(32.))
            .flex_shrink_0()
            .rounded(px(4.))
            .overflow_hidden()
            .bg(colors.tile)
            .flex()
            .items_center()
            .justify_center()
            .map(|tile| match picture {
                Some(image) => tile.child(img(image).size_full().object_fit(ObjectFit::Cover)),
                None => tile.child(svg().path(fallback).size(px(15.)).text_color(colors.muted)),
            });
        let mark = div()
            .size(px(20.))
            .flex_shrink_0()
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .map(|mark| {
                if checked {
                    mark.bg(colors.primary).child(
                        svg()
                            .path(icons::CHECK)
                            .size(px(12.))
                            .text_color(hsla(0., 0., 1., 1.)),
                    )
                } else {
                    mark.border_1().border_color(colors.muted)
                }
            });
        div()
            .id(("save-to", index))
            .h(px(44.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap_3()
            .pl(px(8. + 18. * depth as f32))
            .pr_2()
            .rounded(px(6.))
            .child(tile)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(13.5))
                    .font_weight(if folder {
                        FontWeight::SEMIBOLD
                    } else {
                        FontWeight::MEDIUM
                    })
                    .child(target.name.clone()),
            )
            .when(!folder, |row| {
                row.cursor_pointer()
                    .hover(move |row| row.bg(colors.hover))
                    .child(mark)
                    .on_click(cx.listener(move |this, _, _, cx| this.toggle(&uri, cx)))
            })
    }
}

pub fn curation_changes(
    targets: &[EditTarget],
    chosen: &HashMap<String, bool>,
) -> Vec<(String, bool)> {
    let mut seen = HashSet::new();
    targets
        .iter()
        .filter(|target| target.kind != TargetKind::Folder && seen.insert(&target.uri))
        .filter_map(|target| {
            let wanted = chosen.get(&target.uri).copied().unwrap_or(target.curates);
            (wanted != target.curates).then(|| (target.uri.clone(), wanted))
        })
        .collect()
}

#[derive(Clone, Copy)]
struct Colors {
    muted: Hsla,
    tile: Hsla,
    primary: Hsla,
    hover: Hsla,
}

fn header(text: &'static str, color: Hsla) -> impl IntoElement {
    div()
        .pt_2()
        .pb_1()
        .px_2()
        .text_size(px(12.))
        .font_weight(FontWeight::BOLD)
        .text_color(color)
        .child(text)
}

fn dialog_top(window: &Window) -> f32 {
    (f32::from(window.viewport_size().height) * 0.1).clamp(36., 96.)
}

fn sheet_list_height(window: &Window) -> f32 {
    let height = f32::from(window.viewport_size().height);
    (height - dialog_top(window) - 296.).clamp(120., 360.)
}

impl Render for SaveTo {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (text, border, danger) = (theme.foreground, theme.border, theme.danger);
        let colors = Colors {
            muted: theme.muted_foreground,
            tile: theme.muted,
            primary: theme.primary,
            hover: theme.list_hover,
        };
        let filter = self.search.read(cx).value().trim().to_lowercase();
        let pictures: HashMap<String, Arc<Image>> = self
            .app
            .upgrade()
            .map(|app| {
                let app = app.read(cx);
                app.playlists
                    .editable_for(&self.uris)
                    .and_then(|result| result.as_ref().ok())
                    .into_iter()
                    .flatten()
                    .filter_map(|target| {
                        let url = target.cover.as_deref().filter(|url| trusted_image(url))?;
                        Some((target.uri.clone(), app.covers.get(url)?.clone()))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let list = div()
            .id("save-to-list")
            .max_h(px(sheet_list_height(window)))
            .overflow_y_scroll()
            .flex()
            .flex_col();
        let list = match self.targets(cx) {
            None => list.child(
                div()
                    .p_2()
                    .text_size(px(13.))
                    .text_color(colors.muted)
                    .child("Loading playlists…"),
            ),
            Some(Err(error)) => list.child(
                div()
                    .p_2()
                    .text_size(px(13.))
                    .text_color(danger)
                    .child(error),
            ),
            Some(Ok(targets)) => {
                let tree = editable_tree(&targets);
                let mut rows: Vec<AnyElement> = Vec::new();
                let mut index = 0;
                let mut push = |rows: &mut Vec<AnyElement>,
                                depth: usize,
                                target: &EditTarget,
                                this: &Self,
                                cx: &mut Context<Self>| {
                    let picture = pictures.get(&target.uri).cloned();
                    rows.push(
                        this.row(index, depth, target, picture, colors, cx)
                            .into_any_element(),
                    );
                    index += 1;
                };
                if filter.is_empty() {
                    let saved: Vec<&EditTarget> = tree
                        .iter()
                        .map(|(_, target)| *target)
                        .filter(|target| target.kind != TargetKind::Folder && target.curates)
                        .collect();
                    if !saved.is_empty() {
                        rows.push(header("Saved in", colors.muted).into_any_element());
                        for target in &saved {
                            push(&mut rows, 0, target, self, cx);
                        }
                        rows.push(div().my_1().h(px(1.)).bg(border).into_any_element());
                    }
                    for (depth, target) in &tree {
                        if target.kind != TargetKind::Folder && target.curates {
                            continue;
                        }
                        push(&mut rows, *depth, target, self, cx);
                    }
                } else {
                    for (_, target) in &tree {
                        if target.kind != TargetKind::Folder
                            && target.name.to_lowercase().contains(&filter)
                        {
                            push(&mut rows, 0, target, self, cx);
                        }
                    }
                    if index == 0 {
                        rows.push(
                            div()
                                .p_2()
                                .text_size(px(13.))
                                .text_color(colors.muted)
                                .child(format!("Couldn’t find “{filter}”"))
                                .into_any_element(),
                        );
                    }
                }
                list.children(rows)
            }
        };
        div()
            .flex()
            .flex_col()
            .gap_2()
            .text_color(text)
            .child(Input::new(&self.search))
            .child(
                div()
                    .id("save-to-new")
                    .h(px(44.))
                    .flex()
                    .items_center()
                    .gap_3()
                    .px_2()
                    .rounded(px(6.))
                    .cursor_pointer()
                    .hover(move |row| row.bg(colors.hover))
                    .child(
                        div()
                            .size(px(32.))
                            .rounded(px(4.))
                            .bg(colors.tile)
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(svg().path(icons::PLUS).size(px(15.)).text_color(text)),
                    )
                    .child(
                        div()
                            .text_size(px(13.5))
                            .font_weight(FontWeight::MEDIUM)
                            .child("New playlist"),
                    )
                    .on_click(cx.listener(|this, _, window, cx| this.new_playlist(window, cx))),
            )
            .child(list)
    }
}

pub fn open_save_to(window: &mut Window, cx: &mut App, sheet: Entity<SaveTo>) {
    window.open_dialog(cx, move |dialog, window, cx| {
        let done = sheet.clone();
        dialog
            .title("Add to playlist")
            .w(px(420.))
            .margin_top(px(dialog_top(window)))
            .bg(cx.theme().popover)
            .child(sheet.clone())
            .button_props(
                DialogButtonProps::default()
                    .ok_text("Done")
                    .cancel_text("Cancel"),
            )
            .footer(|ok, cancel, window, cx| vec![cancel(window, cx), ok(window, cx)])
            .on_ok(move |_, _, cx| done.update(cx, |sheet, cx| sheet.done(cx)))
    });
}

#[cfg(test)]
mod tests {
    use spotify_client::playlist::LIKED_SONGS;

    use super::*;

    fn target(uri: &str, kind: TargetKind, curates: bool) -> EditTarget {
        EditTarget {
            uri: uri.into(),
            name: uri.into(),
            kind,
            curates,
            pinned: false,
            cover: None,
            folder: None,
        }
    }

    #[test]
    fn sends_only_what_the_sheet_changed() {
        let targets = vec![
            target(LIKED_SONGS, TargetKind::LikedSongs, true),
            target("spotify:user:someone:folder:f", TargetKind::Folder, false),
            target("spotify:playlist:a", TargetKind::Playlist, false),
            target("spotify:playlist:b", TargetKind::Playlist, true),
            target("spotify:playlist:a", TargetKind::Playlist, false),
        ];
        assert!(curation_changes(&targets, &HashMap::new()).is_empty());
        let chosen = HashMap::from([
            (LIKED_SONGS.to_owned(), false),
            ("spotify:playlist:a".to_owned(), true),
            ("spotify:playlist:b".to_owned(), true),
            ("spotify:user:someone:folder:f".to_owned(), true),
        ]);
        assert_eq!(
            curation_changes(&targets, &chosen),
            [
                (LIKED_SONGS.to_owned(), false),
                ("spotify:playlist:a".to_owned(), true)
            ]
        );
    }
}
