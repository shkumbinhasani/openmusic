use gpui::ScrollHandle;
use spotify_client::catalog::Lyrics;

use crate::app::{OpenMusic, Page};

#[derive(Default)]
pub struct State {
    pub lyrics: Option<(String, Option<Lyrics>)>,
    pub lyrics_scroll: ScrollHandle,
    pub lyrics_line: Option<usize>,
    pub lyrics_loading: bool,
}

impl OpenMusic {
    pub fn current_lyrics(&self) -> Option<&Lyrics> {
        let (uri, lyrics) = self.content.lyrics.as_ref()?;
        if !self.is_current(uri) {
            return None;
        }
        lyrics.as_ref()
    }

    fn lyric_line(&self) -> Option<usize> {
        let lyrics = self.current_lyrics().filter(|lyrics| lyrics.synced)?;
        let position = self.playback.now.position();
        lyrics
            .lines
            .iter()
            .rposition(|line| line.start_ms <= position)
    }

    pub fn follow_lyrics(&mut self) -> bool {
        let line = self.lyric_line();
        if line == self.content.lyrics_line {
            return false;
        }
        self.content.lyrics_line = line;
        if self.page == Page::Lyrics
            && let Some(line) = line
        {
            self.content
                .lyrics_scroll
                .scroll_to_top_of_item((line + 1).saturating_sub(3));
        }
        true
    }

    pub fn show_lyrics(&mut self, show: bool) {
        if show {
            self.navigate(Page::Lyrics);
            self.content.lyrics_line = None;
            self.follow_lyrics();
        } else if self.page == Page::Lyrics {
            self.back();
        }
    }
}
