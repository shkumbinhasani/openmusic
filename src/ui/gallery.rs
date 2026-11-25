use std::{path::Path, rc::Rc};

use gpui::{
    AnyElement, Context, FontWeight, Pixels, SharedString, Window, div, prelude::*, px, rgb,
};
use spotify_client::{
    Track,
    entity::{Card, Kind},
};

use super::{
    Palette, Panel,
    cards::{Face, card},
    components::{Sentinel, scroll_sentinel, skeleton_rows},
    tracks::{RowDrag, RowDrop, RowOptions, now_ms},
};
use crate::{
    app::{OpenMusic, Page},
    overlay,
};

const ROWS: &str = "gallery-track";
const PLAY: &str = "gallery-play";
const MAX_LOADS: usize = 6;
const CHIPS: [&str; 5] = ["All", "Songs", "Artists", "Albums", "Playlists"];

#[derive(Default)]
pub struct Gallery {
    sentinel: Sentinel,
    loads: usize,
    chip: usize,
    presses: usize,
    clicks: Vec<String>,
    drops: Vec<String>,
}

pub fn debug(app: &mut OpenMusic, rest: &str, directory: &Path) {
    match rest {
        "on" => app.gallery = Some(Gallery::default()),
        "off" => app.gallery = None,
        _ => {}
    }
    let text = match &app.gallery {
        Some(gallery) => {
            let mut lines = vec![format!(
                "on=true chip={} loads={} presses={}",
                CHIPS[gallery.chip], gallery.loads, gallery.presses
            )];
            lines.extend(gallery.clicks.iter().map(|click| format!("click {click}")));
            lines.extend(gallery.drops.iter().map(|drop| format!("drop {drop}")));
            lines.join("\n")
        }
        None => "on=false".to_owned(),
    };
    let _ = std::fs::write(directory.join("gallery.txt"), text);
}

pub fn tick(app: &mut OpenMusic, cx: &mut Context<OpenMusic>) {
    let Some(gallery) = app.gallery.as_mut() else {
        return;
    };
    if gallery.sentinel.take() && gallery.loads < MAX_LOADS {
        gallery.loads += 1;
        cx.notify();
    }
}

pub fn on_press(app: &mut OpenMusic, key: &'static str) -> bool {
    match app.gallery.as_mut() {
        Some(gallery) if key == PLAY => {
            gallery.presses += 1;
            true
        }
        _ => false,
    }
}

pub fn row_dropped(
    app: &mut OpenMusic,
    drag: &RowDrag,
    at: RowDrop,
    _window: &mut Window,
    _cx: &mut Context<OpenMusic>,
) -> bool {
    let Some(gallery) = app.gallery.as_mut().filter(|_| at.row.0 == ROWS) else {
        return false;
    };
    gallery.drops.push(format!(
        "{} to={}:{} after={}",
        drag.describe(),
        at.row.0,
        at.row.1,
        at.after
    ));
    true
}

pub fn panel(app: &OpenMusic, palette: Palette) -> Option<Panel> {
    app.gallery.as_ref()?;
    let element = palette
        .glass("gallery-panel", 0.)
        .size_full()
        .flex()
        .flex_col()
        .gap_2()
        .pt(px(48.))
        .px_4()
        .child(
            div()
                .text_size(px(15.))
                .font_weight(FontWeight::BOLD)
                .child("Right panel"),
        )
        .child(
            div()
                .text_size(px(12.5))
                .text_color(rgb(palette.muted))
                .child("Shown only when the window is 1060 px or wider."),
        );
    Some(Panel {
        width: 260.,
        element: element.into_any_element(),
    })
}

fn sample_tracks(app: &OpenMusic) -> Vec<Track> {
    let mut tracks: Vec<Track> = [
        app.discovery.results.as_slice(),
        app.discovery.home.tracks.as_slice(),
    ]
    .into_iter()
    .find(|tracks| !tracks.is_empty())
    .unwrap_or_default()
    .iter()
    .take(4)
    .cloned()
    .collect();
    let now = now_ms();
    let ages = [5 * 60_000, 3 * 3_600_000, 4 * 86_400_000, 400 * 86_400_000];
    for (track, age) in tracks.iter_mut().zip(ages) {
        track.added_at = Some(now - age);
    }
    if let Some(track) = tracks.get_mut(1) {
        track.explicit = true;
    }
    tracks.push(Track {
        uri: "spotify:local:Sample+Artist:Sample+Album:Local+file:200".into(),
        title: "Local file".into(),
        artist: "Sample Artist".into(),
        album: "Sample Album".into(),
        duration_ms: 200_000,
        added_at: Some(now - 60_000),
        ..Track::default()
    });
    tracks
}

fn heading(text: &'static str) -> impl IntoElement {
    div()
        .text_size(px(19.))
        .font_weight(FontWeight::BOLD)
        .child(text)
}

pub fn page(
    app: &OpenMusic,
    card_side: Pixels,
    palette: Palette,
    cx: &mut Context<OpenMusic>,
) -> AnyElement {
    let Some(gallery) = app.gallery.as_ref() else {
        return div().into_any_element();
    };
    let tracks = sample_tracks(app);
    let albums: Vec<Card> = tracks
        .iter()
        .filter_map(|track| {
            let uri = track.album_uri.as_deref()?;
            Some(card(
                uri,
                Kind::Album,
                &track.album,
                &track.artist,
                track.cover.clone(),
            ))
        })
        .cycle()
        .take(8)
        .collect();
    let artists: Vec<Card> = tracks
        .iter()
        .flat_map(|track| track.artists.iter())
        .take(5)
        .map(|artist| card(&artist.uri, Kind::Artist, &artist.name, "Artist", None))
        .collect();
    let mut faces: Vec<Face> = tracks
        .iter()
        .map(|track| Face {
            name: track.artist.clone(),
            image: track.cover.clone(),
        })
        .collect();
    faces.push(Face {
        name: "Untrusted host".into(),
        image: Some("https://example.invalid/avatar.jpg".into()),
    });
    faces.push(Face {
        name: "No picture".into(),
        image: None,
    });
    let hero = tracks.first().map(|track| {
        let mut hero = card(
            &track.uri,
            Kind::Track,
            &track.title,
            &track.artist,
            track.cover.clone(),
        );
        hero.explicit = track.explicit;
        hero
    });
    let chips: Vec<SharedString> = CHIPS.iter().map(|chip| (*chip).into()).collect();
    let options = RowOptions {
        show_added: true,
        editable: true,
        draggable: true,
        on_click: Some(Rc::new(|app, tracks, index, _, _| {
            if let (Some(gallery), Some(track)) = (app.gallery.as_mut(), tracks.get(index)) {
                gallery.clicks.push(format!("{index} {}", track.uri));
            }
        })),
        ..RowOptions::default()
    };
    div()
        .flex()
        .flex_col()
        .gap_7()
        .child(
            div()
                .text_size(px(30.))
                .font_weight(FontWeight::BOLD)
                .child("Components"),
        )
        .child(app.chip_row(
            "gallery-chip",
            &chips,
            Some(gallery.chip),
            |app, index, _, _| {
                if let Some(gallery) = app.gallery.as_mut() {
                    gallery.chip = index;
                }
            },
            palette,
            cx,
        ))
        .children(hero.map(|hero| app.hero_card(&hero, "gallery-hero", Some(PLAY), palette, cx)))
        .child(app.shelf(
            "Shelf",
            Some(Page::Home),
            &albums,
            "gallery-shelf",
            card_side,
            palette,
            cx,
        ))
        .child(
            div()
                .flex()
                .flex_col()
                .gap_3()
                .child(heading("Card grid"))
                .child(app.card_grid(&artists, "gallery-artist", card_side, palette, cx)),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap_3()
                .child(heading("People"))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_6()
                        .child(app.facepile(&faces, px(32.), 4, palette))
                        .children(faces.iter().rev().take(3).zip([28., 40., 64.]).map(
                            |(face, side)| {
                                app.avatar(&face.name, face.image.as_deref(), px(side), palette)
                            },
                        )),
                ),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(heading("Track list"))
                .child(
                    div()
                        .relative()
                        .child(app.track_list(&tracks, ROWS, 0, &options, palette, cx))
                        .child(overlay::anchor("gallery-rows")),
                )
                .child(skeleton_rows(
                    "gallery-more",
                    gallery.loads * 2,
                    false,
                    palette,
                ))
                .child(scroll_sentinel(&gallery.sentinel)),
        )
        .into_any_element()
}
