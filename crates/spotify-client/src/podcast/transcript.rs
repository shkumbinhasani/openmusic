use anyhow::Result;
use reqwest::StatusCode;
use serde_json::Value;

use super::Transcript;
use crate::{
    api::{Api, HttpStatus},
    catalog::{LyricLine, Lyrics},
};

pub const PATH: &str = "/transcript-read-along/v2/episode/";
pub const MAX_LINES: usize = 5000;
pub const SENTENCE_LIMIT: u32 = 500;
pub const CREATOR_NOTE: &str = "Transcript provided by the creator.";
pub const GENERATED_NOTE: &str =
    "This transcript was generated automatically. Its accuracy may vary.";

pub fn pick<'a>(transcripts: &'a [Transcript], language: &str) -> Option<&'a Transcript> {
    let wanted = language.to_ascii_lowercase();
    let base = wanted.split(['-', '_']).next().unwrap_or_default();
    transcripts
        .iter()
        .find(|transcript| transcript.language.eq_ignore_ascii_case(&wanted))
        .or_else(|| {
            transcripts.iter().find(|transcript| {
                !base.is_empty()
                    && transcript
                        .language
                        .split(['-', '_'])
                        .next()
                        .is_some_and(|language| language.eq_ignore_ascii_case(base))
            })
        })
        .or_else(|| transcripts.first())
}

fn episode_id(uri: &str) -> Option<&str> {
    uri.strip_prefix("spotify:episode:")
        .filter(|id| !id.is_empty() && id.bytes().all(|byte| byte.is_ascii_alphanumeric()))
}

pub fn request_path(episode_uri: &str, transcript: Option<&Transcript>) -> Option<String> {
    let id = episode_id(episode_uri)?;
    let path = transcript
        .and_then(|transcript| url::Url::parse(&transcript.url).ok())
        .map(|url| url.path().trim_end_matches('/').to_owned())
        .filter(|path| {
            path.strip_prefix(PATH)
                .and_then(|rest| rest.split('/').next())
                .is_some_and(|found| found == id)
        })
        .unwrap_or_else(|| format!("{PATH}{id}"));
    Some(format!(
        "{path}?format=json&maxSentenceLength={SENTENCE_LIMIT}&excludeCC=true"
    ))
}

fn milliseconds(value: &Value) -> Option<u32> {
    value
        .as_u64()
        .or_else(|| value.as_str()?.parse().ok())
        .map(|ms| ms.min(u64::from(u32::MAX)) as u32)
}

fn words(value: &Value) -> Option<String> {
    let text = value
        .as_str()?
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    (!text.is_empty()).then_some(text)
}

pub fn parse(value: &Value) -> Option<Lyrics> {
    let synced = matches!(
        value["timeSyncedStatus"].as_str(),
        Some("SENTENCE_SYNCED" | "SYLLABLE_SYNCED")
    );
    let lines: Vec<LyricLine> = value["section"]
        .as_array()?
        .iter()
        .filter_map(|section| {
            let start = milliseconds(&section["startMs"]);
            if let Some(title) = words(&section["title"]["title"]) {
                return Some(LyricLine {
                    start_ms: start.unwrap_or_default(),
                    words: title,
                    heading: true,
                });
            }
            let sentence = &section["text"]["sentence"];
            Some(LyricLine {
                start_ms: milliseconds(&sentence["startMs"])
                    .or(start)
                    .unwrap_or_default(),
                words: words(&sentence["text"])?,
                heading: false,
            })
        })
        .take(MAX_LINES)
        .collect();
    if lines.iter().all(|line| line.heading) {
        return None;
    }
    let creator = value["creatorProvided"].as_bool().unwrap_or(false);
    Some(Lyrics {
        synced,
        lines,
        provider: if creator {
            CREATOR_NOTE
        } else {
            GENERATED_NOTE
        }
        .to_owned(),
    })
}

pub async fn fetch(
    api: &Api,
    episode_uri: &str,
    transcript: Option<&Transcript>,
) -> Result<Option<Lyrics>> {
    let Some(path) = request_path(episode_uri, transcript) else {
        return Ok(None);
    };
    match api.get_json::<Value>(&path).await {
        Ok(value) => Ok(parse(&value)),
        Err(error)
            if error
                .downcast_ref::<HttpStatus>()
                .is_some_and(|status| status.0 == StatusCode::NOT_FOUND) =>
        {
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

pub async fn read(
    api: &Api,
    episode_uri: &str,
    known: Option<Vec<Transcript>>,
) -> Result<Option<Lyrics>> {
    let transcripts = match known {
        Some(transcripts) => transcripts,
        None => super::episode(api, episode_uri).await?.transcripts,
    };
    let Some(transcript) = pick(&transcripts, &api.language()) else {
        return Ok(None);
    };
    fetch(api, episode_uri, Some(transcript)).await
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const EPISODE: &str = "spotify:episode:00000000000000000000a1";

    fn transcript(language: &str, url: &str) -> Transcript {
        Transcript {
            language: language.to_owned(),
            url: url.to_owned(),
            fixed: false,
        }
    }

    #[test]
    fn requests_the_read_along_path_on_our_spclient() {
        let english = transcript(
            "en-us",
            "https://spclient.wg.spotify.com/transcript-read-along/v2/episode/00000000000000000000a1",
        );
        assert_eq!(
            request_path(EPISODE, Some(&english)).as_deref(),
            Some(
                "/transcript-read-along/v2/episode/00000000000000000000a1?format=json&maxSentenceLength=500&excludeCC=true"
            )
        );
        let german = transcript(
            "de-de",
            "https://spclient.wg.spotify.com/transcript-read-along/v2/episode/00000000000000000000a1/de-de",
        );
        assert!(
            request_path(EPISODE, Some(&german))
                .unwrap()
                .starts_with("/transcript-read-along/v2/episode/00000000000000000000a1/de-de?")
        );
        let other = transcript(
            "en",
            "https://spclient.wg.spotify.com/transcript-read-along/v2/episode/00000000000000000000b1",
        );
        assert!(
            request_path(EPISODE, Some(&other))
                .unwrap()
                .starts_with("/transcript-read-along/v2/episode/00000000000000000000a1?")
        );
        let elsewhere = transcript("en", "https://spclient.wg.spotify.com/other/path");
        assert!(
            request_path(EPISODE, Some(&elsewhere))
                .unwrap()
                .starts_with("/transcript-read-along/v2/episode/00000000000000000000a1?")
        );
        assert!(request_path(EPISODE, None).is_some());
        assert_eq!(
            request_path("spotify:track:00000000000000000000a1", None),
            None
        );
        assert_eq!(request_path("spotify:episode:../x", None), None);
    }

    #[test]
    fn picks_the_language_or_falls_back() {
        let list = [
            transcript("en-us", "a"),
            transcript("de-de", "b"),
            transcript("pt-br", "c"),
        ];
        assert_eq!(pick(&list, "pt-BR").unwrap().url, "c");
        assert_eq!(pick(&list, "de").unwrap().url, "b");
        assert_eq!(pick(&list, "ja").unwrap().url, "a");
        assert!(pick(&[], "en").is_none());
    }

    fn reply() -> Value {
        json!({
            "version": "1.0",
            "language": "en-us",
            "timeSyncedStatus": "SYLLABLE_SYNCED",
            "creatorProvided": false,
            "section": [
                {"startMs": 190, "title": {"title": "Intro"}},
                {"startMs": 190, "text": {"sentence": {"startMs": 190, "text": "First  sentence\nhere.", "highlight": [{"startMs": 190, "numChars": 2}]}}},
                {"startMs": 4200, "musicClosedCaption": {"text": "Music"}},
                {"startMs": 5100, "text": {"sentence": {"startMs": "5100", "text": "Second one."}}},
                {"startMs": 9000, "text": {"sentence": {"startMs": 9000, "text": "   "}}}
            ]
        })
    }

    #[test]
    fn turns_sections_into_lines() {
        let lyrics = parse(&reply()).unwrap();
        assert!(lyrics.synced);
        assert_eq!(lyrics.provider, GENERATED_NOTE);
        let lines: Vec<(u32, &str, bool)> = lyrics
            .lines
            .iter()
            .map(|line| (line.start_ms, line.words.as_str(), line.heading))
            .collect();
        assert_eq!(
            lines,
            [
                (190, "Intro", true),
                (190, "First sentence here.", false),
                (5100, "Second one.", false),
            ]
        );
    }

    #[test]
    fn reads_plain_and_creator_transcripts() {
        let mut plain = reply();
        plain["timeSyncedStatus"] = json!("NOT_SYNCED");
        plain["creatorProvided"] = json!(true);
        let lyrics = parse(&plain).unwrap();
        assert!(!lyrics.synced);
        assert_eq!(lyrics.provider, CREATOR_NOTE);
        let titles_only = json!({"section": [{"startMs": 0, "title": {"title": "Intro"}}]});
        assert!(parse(&titles_only).is_none());
        assert!(parse(&json!({})).is_none());
    }

    #[test]
    fn caps_long_transcripts() {
        let sections: Vec<Value> = (0..MAX_LINES + 50)
            .map(|index| json!({"startMs": index, "text": {"sentence": {"startMs": index, "text": "Words."}}}))
            .collect();
        let lyrics =
            parse(&json!({"timeSyncedStatus": "SENTENCE_SYNCED", "section": sections})).unwrap();
        assert_eq!(lyrics.lines.len(), MAX_LINES);
    }
}
