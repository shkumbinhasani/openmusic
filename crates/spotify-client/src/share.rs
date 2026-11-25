pub const WEB: &str = "https://open.spotify.com";

const EMBED_ALLOW: &str =
    "autoplay; clipboard-write; encrypted-media; fullscreen; picture-in-picture";
const WIDTH_PIXELS: (u32, u32) = (280, 600);
const WIDTH_PERCENT: (u32, u32) = (40, 100);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Track,
    Album,
    Artist,
    Playlist,
    Show,
    Episode,
    Audiobook,
    User,
}

impl Kind {
    pub fn of(uri: &str) -> Option<Self> {
        web_target(uri).map(|(kind, _)| kind)
    }

    pub fn copy_label(self) -> &'static str {
        match self {
            Self::Track => "Copy link to Song",
            Self::Album => "Copy link to Album",
            Self::Artist => "Copy link to artist",
            Self::Playlist => "Copy link to playlist",
            Self::Show => "Copy link to Podcast",
            Self::Episode => "Copy link to Episode",
            Self::Audiobook => "Copy link to Audiobook",
            Self::User => "Copy link to profile",
        }
    }

    pub fn embed_title(self) -> Option<&'static str> {
        Some(match self {
            Self::Track => "Embed track",
            Self::Album => "Embed album",
            Self::Artist => "Embed artist",
            Self::Playlist => "Embed playlist",
            Self::Show => "Embed show",
            Self::Episode => "Embed episode",
            Self::Audiobook | Self::User => return None,
        })
    }

    pub fn takes_start(self) -> bool {
        matches!(self, Self::Show | Self::Episode)
    }

    fn path(self) -> &'static str {
        match self {
            Self::Track => "track",
            Self::Album => "album",
            Self::Artist => "artist",
            Self::Playlist => "playlist",
            Self::Show => "show",
            Self::Episode => "episode",
            Self::Audiobook => "audiobook",
            Self::User => "user",
        }
    }
}

fn base62(id: &str) -> bool {
    !id.is_empty() && id.bytes().all(|byte| byte.is_ascii_alphanumeric())
}

fn web_target(uri: &str) -> Option<(Kind, &str)> {
    let parts: Vec<&str> = uri.split(':').collect();
    let (kind, id) = match parts.as_slice() {
        ["spotify", "track", id] => (Kind::Track, *id),
        ["spotify", "album", id] => (Kind::Album, *id),
        ["spotify", "artist", id] => (Kind::Artist, *id),
        ["spotify", "playlist", id] | ["spotify", "user", _, "playlist", id] => {
            (Kind::Playlist, *id)
        }
        ["spotify", "show", id] => (Kind::Show, *id),
        ["spotify", "episode", id] => (Kind::Episode, *id),
        ["spotify", "audiobook", id] => (Kind::Audiobook, *id),
        ["spotify", "user", name] if !name.is_empty() => return Some((Kind::User, *name)),
        _ => return None,
    };
    base62(id).then_some((kind, id))
}

pub fn web_path(uri: &str) -> Option<String> {
    let (kind, id) = web_target(uri)?;
    Some(format!("{}/{id}", kind.path()))
}

pub fn share_id() -> String {
    let mut id = format!("{:016x}", rand::random::<u64>());
    id.replace_range(12..13, "4");
    id
}

pub fn link(uri: &str, share_id: &str) -> Option<String> {
    Some(format!("{WEB}/{}?si={share_id}", web_path(uri)?))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Theme {
    #[default]
    Dynamic,
    Dark,
}

impl Theme {
    pub const DARK_SWATCH: u32 = 0x282828;
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Size {
    #[default]
    Normal,
    Compact,
}

impl Size {
    pub fn height(self) -> u32 {
        match self {
            Self::Normal => 352,
            Self::Compact => 152,
        }
    }

    pub fn label(self) -> String {
        let name = match self {
            Self::Normal => "Normal",
            Self::Compact => "Compact",
        };
        format!("{name} ({}px)", self.height())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Width {
    Percent(u32),
    Pixels(u32),
}

impl Default for Width {
    fn default() -> Self {
        Self::Percent(100)
    }
}

impl Width {
    pub fn parse(text: &str) -> Self {
        let Some(number) = leading_integer(&text.replacen("px", "", 1)) else {
            return Self::default();
        };
        if text.contains("px") {
            let (low, high) = WIDTH_PIXELS;
            Self::Pixels(number.clamp(i64::from(low), i64::from(high)) as u32)
        } else {
            let (low, high) = WIDTH_PERCENT;
            Self::Percent(number.clamp(i64::from(low), i64::from(high)) as u32)
        }
    }

    pub fn label(self) -> String {
        match self {
            Self::Percent(value) => format!("{value}%"),
            Self::Pixels(value) => format!("{value}px"),
        }
    }

    pub fn attribute(self) -> String {
        match self {
            Self::Percent(value) => format!("{value}%"),
            Self::Pixels(value) => value.to_string(),
        }
    }
}

fn leading_integer(text: &str) -> Option<i64> {
    let text = text.trim_start();
    let (negative, digits) = match text.as_bytes().first() {
        Some(b'-') => (true, &text[1..]),
        Some(b'+') => (false, &text[1..]),
        _ => (false, text),
    };
    let end = digits
        .find(|character: char| !character.is_ascii_digit())
        .unwrap_or(digits.len());
    let value: i64 = digits[..end].parse().ok()?;
    Some(if negative { -value } else { value })
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Embed {
    pub theme: Theme,
    pub size: Size,
    pub width: Width,
    pub start: Option<u32>,
    pub share_id: Option<String>,
}

pub fn embed_url(uri: &str, embed: &Embed) -> Option<String> {
    let mut url = format!("{WEB}/embed/{}?utm_source=generator", web_path(uri)?);
    if embed.theme == Theme::Dark {
        url.push_str("&theme=0");
    }
    if let Some(start) = embed.start {
        url.push_str(&format!("&t={start}"));
    }
    if let Some(share_id) = &embed.share_id {
        url.push_str(&format!("&si={share_id}"));
    }
    Some(url)
}

pub fn embed_code(uri: &str, embed: &Embed) -> Option<String> {
    Kind::of(uri)?.embed_title()?;
    let url = embed_url(uri, embed)?;
    Some(format!(
        r#"<iframe data-testid="embed-iframe" style="border-radius:12px" src="{url}" width="{}" height="{}" frameBorder="0" allowfullscreen="" allow="{EMBED_ALLOW}" loading="lazy"></iframe>"#,
        embed.width.attribute(),
        embed.size.height(),
    ))
}

pub fn parse_time(text: &str) -> Option<u32> {
    let parts: Vec<&str> = text.split(':').collect();
    if parts.len() > 3 {
        return None;
    }
    let mut seconds: i64 = 0;
    for (part, unit) in parts.iter().rev().zip([1, 60, 3600]) {
        let value = leading_integer(part).filter(|value| *value >= 0)?;
        seconds = seconds.checked_add(value.checked_mul(unit)?)?;
    }
    u32::try_from(seconds).ok()
}

pub fn format_time(seconds: u32) -> String {
    let mut left = seconds;
    let mut parts: Vec<String> = Vec::new();
    for (index, unit) in [3600, 60, 1].into_iter().enumerate() {
        if left >= unit || !parts.is_empty() || index == 1 {
            let value = left / unit;
            left -= value * unit;
            let width = if parts.is_empty() { 1 } else { 2 };
            parts.push(format!("{value:0width$}"));
        }
    }
    parts.join(":")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_uris_to_web_paths() {
        let cases = [
            (
                "spotify:track:70LcF31zb1H0PyJoS1Sx1r",
                Some("track/70LcF31zb1H0PyJoS1Sx1r"),
            ),
            (
                "spotify:album:1b5KESuOMQomF3wntoLAsk",
                Some("album/1b5KESuOMQomF3wntoLAsk"),
            ),
            (
                "spotify:artist:4Z8W4fKeB5YxbusRsdQVPb",
                Some("artist/4Z8W4fKeB5YxbusRsdQVPb"),
            ),
            (
                "spotify:playlist:37i9dQZF1DXcBWIGoYBM5M",
                Some("playlist/37i9dQZF1DXcBWIGoYBM5M"),
            ),
            (
                "spotify:user:someone:playlist:37i9dQZF1DXcBWIGoYBM5M",
                Some("playlist/37i9dQZF1DXcBWIGoYBM5M"),
            ),
            (
                "spotify:show:0123456789abcdefghijkl",
                Some("show/0123456789abcdefghijkl"),
            ),
            (
                "spotify:episode:0123456789abcdefghijkl",
                Some("episode/0123456789abcdefghijkl"),
            ),
            (
                "spotify:audiobook:0123456789abcdefghijkl",
                Some("audiobook/0123456789abcdefghijkl"),
            ),
            ("spotify:user:someone", Some("user/someone")),
            ("spotify:local:Artist:Album:Title:200", None),
            ("spotify:collection:tracks", None),
            ("spotify:user:someone:collection", None),
            ("spotify:station:track:70LcF31zb1H0PyJoS1Sx1r", None),
            ("spotify:track:", None),
            ("spotify:track:bad/id", None),
            ("https://example.com", None),
        ];
        for (uri, path) in cases {
            assert_eq!(web_path(uri).as_deref(), path, "{uri}");
        }
    }

    #[test]
    fn labels_follow_the_kind() {
        let labels = [
            (
                "spotify:track:70LcF31zb1H0PyJoS1Sx1r",
                "Copy link to Song",
                Some("Embed track"),
            ),
            (
                "spotify:album:1b5KESuOMQomF3wntoLAsk",
                "Copy link to Album",
                Some("Embed album"),
            ),
            (
                "spotify:artist:4Z8W4fKeB5YxbusRsdQVPb",
                "Copy link to artist",
                Some("Embed artist"),
            ),
            (
                "spotify:playlist:37i9dQZF1DXcBWIGoYBM5M",
                "Copy link to playlist",
                Some("Embed playlist"),
            ),
            (
                "spotify:show:0123456789abcdefghijkl",
                "Copy link to Podcast",
                Some("Embed show"),
            ),
            (
                "spotify:episode:0123456789abcdefghijkl",
                "Copy link to Episode",
                Some("Embed episode"),
            ),
            (
                "spotify:audiobook:0123456789abcdefghijkl",
                "Copy link to Audiobook",
                None,
            ),
            ("spotify:user:someone", "Copy link to profile", None),
        ];
        for (uri, copy, embed) in labels {
            let kind = Kind::of(uri).expect(uri);
            assert_eq!(kind.copy_label(), copy);
            assert_eq!(kind.embed_title(), embed);
        }
        assert!(Kind::Episode.takes_start());
        assert!(Kind::Show.takes_start());
        assert!(!Kind::Track.takes_start());
    }

    #[test]
    fn share_ids_look_like_spotify_ones() {
        let first = share_id();
        let second = share_id();
        for id in [&first, &second] {
            assert_eq!(id.len(), 16);
            assert!(id.bytes().all(|byte| byte.is_ascii_hexdigit()));
            assert_eq!(&id[12..13], "4");
        }
        assert_ne!(first, second);
    }

    #[test]
    fn builds_share_links() {
        assert_eq!(
            link("spotify:track:70LcF31zb1H0PyJoS1Sx1r", "0123456789ab4def").as_deref(),
            Some("https://open.spotify.com/track/70LcF31zb1H0PyJoS1Sx1r?si=0123456789ab4def")
        );
        assert_eq!(
            link("spotify:user:someone", "0123456789ab4def").as_deref(),
            Some("https://open.spotify.com/user/someone?si=0123456789ab4def")
        );
        assert_eq!(link("spotify:collection:tracks", "0123456789ab4def"), None);
    }

    #[test]
    fn builds_the_default_embed_code() {
        let code = embed_code("spotify:track:70LcF31zb1H0PyJoS1Sx1r", &Embed::default());
        assert_eq!(
            code.as_deref(),
            Some(
                r#"<iframe data-testid="embed-iframe" style="border-radius:12px" src="https://open.spotify.com/embed/track/70LcF31zb1H0PyJoS1Sx1r?utm_source=generator" width="100%" height="352" frameBorder="0" allowfullscreen="" allow="autoplay; clipboard-write; encrypted-media; fullscreen; picture-in-picture" loading="lazy"></iframe>"#
            )
        );
    }

    #[test]
    fn embed_options_change_the_url_and_size() {
        let embed = Embed {
            theme: Theme::Dark,
            size: Size::Compact,
            width: Width::Pixels(300),
            start: Some(95),
            share_id: Some("0123456789ab4def".into()),
        };
        let uri = "spotify:episode:0123456789abcdefghijkl";
        assert_eq!(
            embed_url(uri, &embed).as_deref(),
            Some(
                "https://open.spotify.com/embed/episode/0123456789abcdefghijkl?utm_source=generator&theme=0&t=95&si=0123456789ab4def"
            )
        );
        let code = embed_code(uri, &embed).unwrap();
        assert!(code.contains(r#" width="300" height="152" "#), "{code}");
        assert_eq!(embed_code("spotify:user:someone", &embed), None);
        assert_eq!(embed_code("spotify:collection:tracks", &embed), None);
    }

    #[test]
    fn widths_follow_the_embed_constraints() {
        assert_eq!(Width::parse("100%"), Width::Percent(100));
        assert_eq!(Width::parse("80%"), Width::Percent(80));
        assert_eq!(Width::parse("10%"), Width::Percent(40));
        assert_eq!(Width::parse("300"), Width::Percent(100));
        assert_eq!(Width::parse("50"), Width::Percent(50));
        assert_eq!(Width::parse("300px"), Width::Pixels(300));
        assert_eq!(Width::parse("100px"), Width::Pixels(280));
        assert_eq!(Width::parse("900px"), Width::Pixels(600));
        assert_eq!(Width::parse(" 320px "), Width::Pixels(320));
        assert_eq!(Width::parse("wide"), Width::Percent(100));
        assert_eq!(Width::parse(""), Width::Percent(100));
        assert_eq!(Width::Pixels(320).label(), "320px");
        assert_eq!(Width::Pixels(320).attribute(), "320");
        assert_eq!(Width::Percent(80).attribute(), "80%");
    }

    #[test]
    fn reads_and_writes_start_times() {
        assert_eq!(parse_time("0:00"), Some(0));
        assert_eq!(parse_time("1:05"), Some(65));
        assert_eq!(parse_time("95"), Some(95));
        assert_eq!(parse_time("1:02:05"), Some(3725));
        assert_eq!(parse_time("1:2:3:4"), None);
        assert_eq!(parse_time("a:10"), None);
        assert_eq!(parse_time("-1:10"), None);
        assert_eq!(parse_time("99999999999999999:00:00"), None);
        assert_eq!(parse_time("999999999999999999:00"), None);
        assert_eq!(parse_time("5124095576030432:00:00"), None);
        assert_eq!(parse_time("1:9223372036854775807"), None);
        assert_eq!(parse_time("1193046:28:15"), Some(u32::MAX));
        assert_eq!(parse_time("1193046:28:16"), None);
        assert_eq!(parse_time(""), None);
        assert_eq!(format_time(0), "0:00");
        assert_eq!(format_time(5), "0:05");
        assert_eq!(format_time(65), "1:05");
        assert_eq!(format_time(600), "10:00");
        assert_eq!(format_time(3725), "1:02:05");
        assert_eq!(Size::Normal.label(), "Normal (352px)");
        assert_eq!(Size::Compact.label(), "Compact (152px)");
    }
}
