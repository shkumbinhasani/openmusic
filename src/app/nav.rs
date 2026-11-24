use crate::{app::OpenMusic, worker::Command};

#[derive(Clone, PartialEq)]
pub enum Page {
    Home,
    Search,
    Lyrics,
    Artist(String),
    Collection(String),
}

impl OpenMusic {
    pub fn navigate(&mut self, page: Page) {
        if self.page == page {
            return;
        }
        match &page {
            Page::Artist(uri)
                if self
                    .discovery
                    .artist
                    .as_ref()
                    .is_none_or(|(loaded, _)| loaded != uri) =>
            {
                self.discovery.artist = None;
                self.send(Command::Artist(uri.clone()));
            }
            Page::Collection(uri)
                if self
                    .playlists
                    .collection
                    .as_ref()
                    .is_none_or(|(loaded, _)| loaded != uri) =>
            {
                self.playlists.collection = None;
                self.send(Command::Collection(uri.clone()));
            }
            _ => {}
        }
        let previous = std::mem::replace(&mut self.page, page);
        self.history.push(previous);
        if self.history.len() > 30 {
            self.history.remove(0);
        }
    }

    pub fn back(&mut self) {
        let page = self.history.pop().unwrap_or(Page::Home);
        match &page {
            Page::Artist(uri)
                if self
                    .discovery
                    .artist
                    .as_ref()
                    .is_none_or(|(loaded, _)| loaded != uri) =>
            {
                self.send(Command::Artist(uri.clone()));
            }
            Page::Collection(uri)
                if self
                    .playlists
                    .collection
                    .as_ref()
                    .is_none_or(|(loaded, _)| loaded != uri) =>
            {
                self.send(Command::Collection(uri.clone()));
            }
            _ => {}
        }
        self.page = page;
    }
}
