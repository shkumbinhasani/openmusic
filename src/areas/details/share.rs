use gpui::{ClipboardItem, Context};
use spotify_client::share::{self, Kind};

use crate::{app::OpenMusic, ui::menu::MenuItem};

pub const LINK_COPIED: &str = "Link copied to clipboard";
pub const URI_COPIED: &str = "URI copied to clipboard";

pub fn menu_item(uri: &str) -> Option<MenuItem> {
    let kind = Kind::of(uri)?;
    let copy = copy_item(uri, kind);
    if kind == Kind::User {
        return Some(copy);
    }
    Some(MenuItem::submenu("Share", vec![copy]).symbol("square.and.arrow.up"))
}

fn copy_item(uri: &str, kind: Kind) -> MenuItem {
    let link = uri.to_owned();
    let plain = uri.to_owned();
    MenuItem::action(kind.copy_label(), move |app, _, cx| {
        copy_link(app, &link, cx)
    })
    .key("c")
    .symbol("link")
    .alternate("Copy Spotify URI", move |app, _, cx| {
        copy_uri(app, &plain, cx)
    })
}

pub fn copy_link(app: &mut OpenMusic, uri: &str, cx: &mut Context<OpenMusic>) {
    let Some(link) = share::link(uri, &share::share_id()) else {
        return;
    };
    cx.write_to_clipboard(ClipboardItem::new_string(link));
    app.show_toast(LINK_COPIED, None);
}

pub fn copy_uri(app: &mut OpenMusic, uri: &str, cx: &mut Context<OpenMusic>) {
    cx.write_to_clipboard(ClipboardItem::new_string(uri.to_owned()));
    app.show_toast(URI_COPIED, None);
}
