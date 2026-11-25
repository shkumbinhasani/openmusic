use std::sync::Arc;

use gpui::{
    App, Context, Entity, FontWeight, Hsla, Image, ImageFormat, ObjectFit, PathPromptOptions,
    SharedString, WeakEntity, Window, div, hsla, img, prelude::*, px, svg,
};
use gpui_component::{
    ActiveTheme, Sizable, WindowExt,
    button::{Button, ButtonVariants},
    dialog::DialogButtonProps,
    input::{Input, InputState},
};
use spotify_client::playlist::{Details, Level, Member};

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
