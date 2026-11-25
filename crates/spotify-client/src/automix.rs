use std::{collections::HashMap, f32::consts::FRAC_PI_2};

use base64::{
    Engine as _,
    engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD},
};
use prost::Message as _;
use spotify_proto::automix::proto::{AutomationCurves, Preset, PresetType, Transition};

pub const RECIPE: &str = "automix.auto_transition_recipe";
pub const ITEM_SPEED: &str = "item.speed";
pub const MIX: &str = "mix";
pub const REPORTING: &str = "custom_reporting_attribution";
pub const MIXED_PLAYLIST: &str = "MixedPlaylist";
pub const MAX_OVERLAP_MS: u32 = 60_000;
pub const UNMATCHED_MS: u32 = 6_000;
pub const MIN_TAIL_MS: u32 = 1_000;
const SPEED_SLACK: f64 = 0.005;
const TEMPO_SLACK: f32 = 0.01;
const CUT_BLEND: f32 = 0.02;
const BEZIER_SLACK: f32 = 0.01;

#[derive(Clone, Debug, PartialEq)]
pub struct Curve {
    segments: Vec<Segment>,
}

#[derive(Clone, Debug, PartialEq)]
struct Segment {
    start: f32,
    end: f32,
    points: Vec<(f32, f32)>,
}

fn unit(value: f64) -> Option<f32> {
    value.is_finite().then(|| value.clamp(0., 1.) as f32)
}

fn cubic(from: f32, first: f32, second: f32, to: f32, s: f32) -> f32 {
    let rest = 1. - s;
    rest * rest * rest * from
        + 3. * rest * rest * s * first
        + 3. * rest * s * s * second
        + s * s * s * to
}

impl Segment {
    fn first(&self) -> f32 {
        self.points.first().map_or(0., |point| point.1)
    }

    fn last(&self) -> f32 {
        self.points.last().map_or(0., |point| point.1)
    }

    fn bezier(&self) -> bool {
        let near = |x: f32, at: f32| (x - at).abs() <= BEZIER_SLACK;
        matches!(
            self.points.as_slice(),
            [a, b, c, d] if near(a.0, 0.) && near(b.0, 1. / 3.) && near(c.0, 2. / 3.) && near(d.0, 1.)
        )
    }

    fn value(&self, s: f32) -> f32 {
        let s = s.clamp(0., 1.);
        if self.bezier() {
            let [a, b, c, d] = [0, 1, 2, 3].map(|index| self.points[index].1);
            return cubic(a, b, c, d, s);
        }
        let mut previous = match self.points.first() {
            Some(first) if s <= first.0 => return first.1,
            Some(first) => *first,
            None => return 0.,
        };
        for point in &self.points[1..] {
            if s <= point.0 {
                let span = point.0 - previous.0;
                if span <= f32::EPSILON {
                    return point.1;
                }
                return previous.1 + (point.1 - previous.1) * (s - previous.0) / span;
            }
            previous = *point;
        }
        previous.1
    }
}

impl Curve {
    pub fn from_proto(curves: &AutomationCurves) -> Option<Self> {
        let mut segments: Vec<Segment> = curves
            .curves
            .iter()
            .filter_map(|curve| {
                let start = unit(curve.start)?;
                let end = unit(curve.end)?.max(start);
                let mut points: Vec<(f32, f32)> = curve
                    .points
                    .iter()
                    .filter_map(|point| Some((unit(point.x)?, unit(point.y)?)))
                    .collect();
                points.sort_by(|a, b| a.0.total_cmp(&b.0));
                (!points.is_empty()).then_some(Segment { start, end, points })
            })
            .collect();
        segments.sort_by(|a, b| a.start.total_cmp(&b.start).then(a.end.total_cmp(&b.end)));
        (!segments.is_empty()).then_some(Self { segments })
    }

    pub fn at(&self, progress: f32) -> f32 {
        let t = progress.clamp(0., 1.);
        let mut value = self.segments.first().map_or(0., Segment::first);
        for segment in &self.segments {
            if t < segment.start {
                break;
            }
            if t <= segment.end && segment.end > segment.start {
                return segment.value((t - segment.start) / (segment.end - segment.start));
            }
            value = segment.last();
        }
        value
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Fader {
    FadeOut,
    FadeIn,
    CutOut,
    CutIn,
    Drawn(Curve),
}

fn cut(t: f32) -> f32 {
    ((t - (0.5 - CUT_BLEND)) / (2. * CUT_BLEND)).clamp(0., 1.)
}

impl Fader {
    pub fn level(&self, progress: f32) -> f32 {
        let t = progress.clamp(0., 1.);
        let level = match self {
            Self::FadeOut => (t * FRAC_PI_2).cos(),
            Self::FadeIn => (t * FRAC_PI_2).sin(),
            Self::CutOut => (cut(t) * FRAC_PI_2).cos(),
            Self::CutIn => (cut(t) * FRAC_PI_2).sin(),
            Self::Drawn(curve) => curve.at(t),
        };
        level.clamp(0., 1.)
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::FadeOut | Self::FadeIn => "fade",
            Self::CutOut | Self::CutIn => "cut",
            Self::Drawn(_) => "curve",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Shape {
    pub out: Fader,
    pub into: Fader,
}

impl Default for Shape {
    fn default() -> Self {
        Self::fade()
    }
}

impl Shape {
    pub fn fade() -> Self {
        Self {
            out: Fader::FadeOut,
            into: Fader::FadeIn,
        }
    }

    pub fn cut() -> Self {
        Self {
            out: Fader::CutOut,
            into: Fader::CutIn,
        }
    }

    pub fn levels(&self, progress: f32) -> (f32, f32) {
        (self.out.level(progress), self.into.level(progress))
    }

    pub fn name(&self) -> String {
        format!("{}/{}", self.out.name(), self.into.name())
    }
}

fn shape_of(preset: Option<&Preset>) -> Shape {
    let kind = preset
        .and_then(|preset| PresetType::try_from(preset.r#type).ok())
        .unwrap_or(PresetType::Unspecified);
    let base = match kind {
        PresetType::Cut | PresetType::BeatmatchedCut => Shape::cut(),
        PresetType::Unspecified | PresetType::Fade | PresetType::BeatmatchedFade => Shape::fade(),
    };
    let drawn =
        |curves: Option<&AutomationCurves>| curves.and_then(Curve::from_proto).map(Fader::Drawn);
    Shape {
        out: drawn(preset.and_then(|preset| preset.volume_out_curve_override.as_ref()))
            .unwrap_or(base.out),
        into: drawn(preset.and_then(|preset| preset.volume_in_curve_override.as_ref()))
            .unwrap_or(base.into),
    }
}

pub fn plain_speed(speed: f64) -> bool {
    speed == 0. || (speed - 1.).abs() <= SPEED_SLACK
}

pub fn item_speed(attribute: Option<&str>) -> f64 {
    attribute
        .and_then(|text| text.trim().parse::<f64>().ok())
        .filter(|speed| speed.is_finite() && *speed > 0.)
        .unwrap_or(1.)
}

pub fn mixed_list(attributes: &HashMap<String, String>) -> bool {
    attributes.get(MIX).is_some_and(|value| value == "true")
}

pub fn mixed_item(attributes: &HashMap<String, String>) -> bool {
    attributes.contains_key(RECIPE)
        || attributes
            .get(REPORTING)
            .is_some_and(|value| value == MIXED_PLAYLIST)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Side<'a> {
    pub uris: &'a [&'a str],
    pub uid: Option<&'a str>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Recipe {
    pub track_a: Vec<String>,
    pub track_b: Vec<String>,
    pub rows: (String, String),
    pub start_a_ms: i32,
    pub start_b_ms: i32,
    pub duration_ms: i32,
    pub beatmatched: bool,
    pub speed_b: f32,
    pub item_speeds: (f64, f64),
    pub shape: Shape,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    pub at_ms: u32,
    pub length_ms: u32,
    pub lead_in_ms: u32,
    pub start_b_ms: u32,
    pub shape: Shape,
    pub shortened: bool,
}

impl Plan {
    pub fn describe(&self) -> String {
        format!(
            "at={} length={} lead_in={} b_start={} shape={}{}",
            self.at_ms,
            self.length_ms,
            self.lead_in_ms,
            self.start_b_ms,
            self.shape.name(),
            if self.shortened { " shortened" } else { "" }
        )
    }
}

fn uris(first: &str, second: &str) -> Vec<String> {
    [first, second]
        .into_iter()
        .filter(|uri| !uri.is_empty())
        .map(str::to_owned)
        .collect()
}

fn base64(text: &str) -> Option<Vec<u8>> {
    let text = text.trim();
    [STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD]
        .iter()
        .find_map(|engine| engine.decode(text).ok())
}

pub fn decode(text: &str) -> Option<Recipe> {
    let bytes = base64(text)?;
    let transition = Transition::decode(bytes.as_slice()).ok()?;
    let overlap = transition.overlap.as_ref()?;
    Some(Recipe {
        track_a: uris(&overlap.track_a_uri, &overlap.track_a_playable_uri),
        track_b: uris(&overlap.track_b_uri, &overlap.track_b_playable_uri),
        rows: (
            overlap.track_a_row_id.clone(),
            overlap.track_b_row_id.clone(),
        ),
        start_a_ms: overlap.start_a_ms,
        start_b_ms: overlap.start_b_ms,
        duration_ms: overlap.duration_ms,
        beatmatched: overlap.is_beatmatched,
        speed_b: overlap
            .speed_b
            .filter(|speed| speed.is_finite() && *speed > 0.)
            .unwrap_or(1.),
        item_speeds: (overlap.item_speed_a, overlap.item_speed_b),
        shape: shape_of(transition.preset.as_ref()),
    })
}

fn names(recipe: &[String], row: &str, side: Side) -> bool {
    if !recipe.is_empty() {
        return recipe.iter().any(|uri| side.uris.contains(&uri.as_str()));
    }
    !row.is_empty() && side.uid == Some(row)
}

impl Recipe {
    pub fn fits(&self, a: Side, b: Side) -> bool {
        names(&self.track_a, &self.rows.0, a) && names(&self.track_b, &self.rows.1, b)
    }

    pub fn plain_speeds(&self) -> bool {
        plain_speed(self.item_speeds.0) && plain_speed(self.item_speeds.1)
    }

    pub fn plan(&self, a_duration_ms: u32, b_duration_ms: u32) -> Option<Plan> {
        if self.duration_ms <= 0 || self.start_a_ms < 0 {
            return None;
        }
        let mut length = u32::try_from(self.duration_ms).ok()?;
        if length > MAX_OVERLAP_MS {
            return None;
        }
        let mut at = u32::try_from(self.start_a_ms).ok()?;
        let mut start_b = i64::from(self.start_b_ms);
        let shortened =
            self.beatmatched && (self.speed_b - 1.).abs() > TEMPO_SLACK && length > UNMATCHED_MS;
        if shortened {
            let trim = length - UNMATCHED_MS;
            at = at.checked_add(trim)?;
            start_b += i64::from(trim);
            length = UNMATCHED_MS;
        }
        let (start_b_ms, lead_in_ms) = if start_b >= 0 {
            (u32::try_from(start_b).ok()?, 0)
        } else {
            (0, u32::try_from(-start_b).ok()?)
        };
        if lead_in_ms >= length {
            return None;
        }
        if a_duration_ms > 0 && at.saturating_add(MIN_TAIL_MS) > a_duration_ms {
            return None;
        }
        if b_duration_ms > 0 && start_b_ms.saturating_add(length - lead_in_ms) > b_duration_ms {
            return None;
        }
        Some(Plan {
            at_ms: at,
            length_ms: length,
            lead_in_ms,
            start_b_ms,
            shape: self.shape.clone(),
            shortened,
        })
    }
}

#[cfg(test)]
mod tests {
    use spotify_proto::automix::proto::{AutomationCurve, Overlap, Point};

    use super::*;

    const A: &str = "spotify:track:0000000000000000000001";
    const B: &str = "spotify:track:0000000000000000000002";
    const B_PLAYABLE: &str = "spotify:track:0000000000000000000003";

    fn curve(start: f64, end: f64, points: &[(f64, f64)]) -> AutomationCurve {
        AutomationCurve {
            points: points.iter().map(|&(x, y)| Point { x, y }).collect(),
            start,
            end,
        }
    }

    fn curves(list: Vec<AutomationCurve>) -> AutomationCurves {
        AutomationCurves {
            curves: list,
            minimum: 0.,
            maximum: 1.,
        }
    }

    fn overlap() -> Overlap {
        Overlap {
            track_a_row_id: "00000000000000a1".into(),
            track_b_row_id: "00000000000000b2".into(),
            start_a_ms: 180_000,
            start_b_ms: 12_000,
            duration_ms: 8_000,
            track_a_uri: A.into(),
            track_b_uri: B.into(),
            track_b_playable_uri: B_PLAYABLE.into(),
            item_speed_a: 1.,
            item_speed_b: 1.,
            ..Overlap::default()
        }
    }

    fn encoded(transition: &Transition) -> String {
        STANDARD.encode(transition.encode_to_vec())
    }

    #[test]
    fn reads_a_recipe_from_the_playlist_attribute() {
        let transition = Transition {
            overlap: Some(overlap()),
            preset: Some(Preset {
                id: 7,
                r#type: PresetType::Fade as i32,
                ..Preset::default()
            }),
            ..Transition::default()
        };
        let recipe = decode(&encoded(&transition)).unwrap();
        assert_eq!(recipe.track_a, vec![A.to_owned()]);
        assert_eq!(recipe.track_b, vec![B.to_owned(), B_PLAYABLE.to_owned()]);
        assert_eq!(
            (recipe.start_a_ms, recipe.start_b_ms, recipe.duration_ms),
            (180_000, 12_000, 8_000)
        );
        assert_eq!(recipe.shape, Shape::fade());
        assert!(recipe.plain_speeds());
        let side = |uris: &'static [&'static str]| Side { uris, uid: None };
        assert!(recipe.fits(side(&[A]), side(&[B_PLAYABLE])));
        assert!(recipe.fits(side(&["spotify:track:x", A]), side(&[B])));
        assert!(!recipe.fits(side(&[B]), side(&[A])));
        assert!(!recipe.fits(side(&[A]), side(&["spotify:track:x"])));
        assert!(
            !recipe.fits(
                Side {
                    uris: &[],
                    uid: Some("00000000000000a1")
                },
                side(&[B])
            ),
            "uris decide when the recipe names them"
        );

        let by_row = Recipe {
            track_a: Vec::new(),
            track_b: Vec::new(),
            ..recipe.clone()
        };
        let row = |uid: &'static str| Side {
            uris: &[A, B],
            uid: Some(uid),
        };
        assert!(by_row.fits(row("00000000000000a1"), row("00000000000000b2")));
        assert!(!by_row.fits(row("00000000000000a1"), row("00000000000000ff")));
        assert!(!by_row.fits(side(&[A]), side(&[B])));
        let nameless = Recipe {
            rows: (String::new(), String::new()),
            ..by_row
        };
        assert!(!nameless.fits(Side::default(), Side::default()));

        let unpadded = encoded(&transition).trim_end_matches('=').to_owned();
        assert_eq!(decode(&unpadded), Some(recipe));
        assert_eq!(decode("not base64 at all!"), None);
        assert_eq!(decode(&STANDARD.encode([0xff, 0xff, 0xff])), None);
        assert_eq!(
            decode(&encoded(&Transition::default())),
            None,
            "a recipe without an overlap means nothing"
        );
    }

    #[test]
    fn takes_the_volume_curves_when_the_recipe_draws_them() {
        let transition = Transition {
            overlap: Some(overlap()),
            preset: Some(Preset {
                r#type: PresetType::Cut as i32,
                volume_out_curve_override: Some(curves(vec![curve(0., 1., &[(0., 1.), (1., 0.)])])),
                ..Preset::default()
            }),
            ..Transition::default()
        };
        let recipe = decode(&encoded(&transition)).unwrap();
        assert!(matches!(recipe.shape.out, Fader::Drawn(_)));
        assert_eq!(recipe.shape.into, Fader::CutIn);
        assert_eq!(recipe.shape.name(), "curve/cut");
        assert!((recipe.shape.out.level(0.25) - 0.75).abs() < 1e-6);

        let empty = Transition {
            overlap: Some(overlap()),
            preset: Some(Preset {
                r#type: PresetType::BeatmatchedCut as i32,
                volume_in_curve_override: Some(curves(Vec::new())),
                ..Preset::default()
            }),
            ..Transition::default()
        };
        assert_eq!(decode(&encoded(&empty)).unwrap().shape, Shape::cut());
        let unknown = Transition {
            overlap: Some(overlap()),
            preset: Some(Preset {
                r#type: 99,
                ..Preset::default()
            }),
            ..Transition::default()
        };
        assert_eq!(decode(&encoded(&unknown)).unwrap().shape, Shape::fade());
    }

    #[test]
    fn follows_curves_like_the_desktop_editor_draws_them() {
        let drawn = Curve::from_proto(&curves(vec![
            curve(0.5, 1., &[(0., 0.2), (1., 0.)]),
            curve(0.5, 0.5, &[(0., 1.), (1., 0.2)]),
            curve(0., 0.5, &[(0., 1.), (1. / 3., 1.), (2. / 3., 1.), (1., 1.)]),
        ]))
        .unwrap();
        assert_eq!(drawn.at(0.), 1.);
        assert_eq!(drawn.at(0.25), 1.);
        assert_eq!(drawn.at(0.5), 1.);
        assert!((drawn.at(0.51) - 0.196).abs() < 1e-3);
        assert!((drawn.at(0.75) - 0.1).abs() < 1e-6);
        assert_eq!(drawn.at(1.), 0.);
        assert_eq!(drawn.at(7.), 0.);
        assert_eq!(drawn.at(-1.), 1.);

        let bend = Curve::from_proto(&curves(vec![curve(
            0.,
            1.,
            &[(0., 0.), (1. / 3., 1.), (2. / 3., 1.), (1., 1.)],
        )]))
        .unwrap();
        assert!((bend.at(0.5) - 0.875).abs() < 1e-6);

        let late =
            Curve::from_proto(&curves(vec![curve(0.25, 0.75, &[(0., 0.), (1., 1.)])])).unwrap();
        assert_eq!(late.at(0.1), 0.);
        assert!((late.at(0.5) - 0.5).abs() < 1e-6);
        assert_eq!(late.at(0.9), 1.);

        let wild = Curve::from_proto(&curves(vec![curve(-3., f64::NAN, &[(0., 0.)])]));
        assert_eq!(wild, None);
        let clamped =
            Curve::from_proto(&curves(vec![curve(0., 1., &[(0., 4.), (1., -2.)])])).unwrap();
        assert_eq!(clamped.at(0.), 1.);
        assert_eq!(clamped.at(1.), 0.);
    }

    #[test]
    fn fades_with_equal_power_and_cuts_in_the_middle() {
        let fade = Shape::fade();
        for step in 0..=20 {
            let (out, into) = fade.levels(step as f32 / 20.);
            assert!((out * out + into * into - 1.).abs() < 1e-5);
        }
        assert_eq!(fade.levels(0.), (1., 0.));
        let (out, into) = fade.levels(1.);
        assert!(out < 1e-6 && (into - 1.).abs() < 1e-6);

        let cut = Shape::cut();
        assert_eq!(cut.levels(0.), (1., 0.));
        assert_eq!(cut.levels(0.47), (1., 0.));
        let (out, into) = cut.levels(0.5);
        assert!((out - into).abs() < 1e-6);
        let (out, into) = cut.levels(0.53);
        assert!(out < 1e-6 && (into - 1.).abs() < 1e-6);
    }

    fn recipe() -> Recipe {
        decode(&encoded(&Transition {
            overlap: Some(overlap()),
            ..Transition::default()
        }))
        .unwrap()
    }

    #[test]
    fn plans_the_overlap_inside_both_songs() {
        let plan = recipe().plan(200_000, 240_000).unwrap();
        assert_eq!(
            (plan.at_ms, plan.length_ms, plan.lead_in_ms, plan.start_b_ms),
            (180_000, 8_000, 0, 12_000)
        );
        assert!(!plan.shortened);
        assert_eq!(
            plan.describe(),
            "at=180000 length=8000 lead_in=0 b_start=12000 shape=fade/fade"
        );

        assert!(recipe().plan(181_000, 240_000).is_some());
        assert_eq!(recipe().plan(180_999, 240_000), None);
        assert_eq!(recipe().plan(200_000, 19_999), None);
        assert!(recipe().plan(200_000, 20_000).is_some());
        assert!(recipe().plan(0, 0).is_some());

        let late_b = Recipe {
            start_b_ms: -3_000,
            ..recipe()
        };
        let plan = late_b.plan(200_000, 240_000).unwrap();
        assert_eq!((plan.start_b_ms, plan.lead_in_ms), (0, 3_000));
        let never = Recipe {
            start_b_ms: -8_000,
            ..recipe()
        };
        assert_eq!(never.plan(200_000, 240_000), None);

        for broken in [
            Recipe {
                duration_ms: 0,
                ..recipe()
            },
            Recipe {
                start_a_ms: -1,
                ..recipe()
            },
            Recipe {
                duration_ms: 60_001,
                ..recipe()
            },
        ] {
            assert_eq!(broken.plan(200_000, 240_000), None);
        }
    }

    #[test]
    fn shortens_an_overlap_that_needs_beatmatching() {
        let matched = Recipe {
            beatmatched: true,
            speed_b: 1.04,
            duration_ms: 16_000,
            start_a_ms: 170_000,
            start_b_ms: 4_000,
            ..recipe()
        };
        let plan = matched.plan(200_000, 240_000).unwrap();
        assert!(plan.shortened);
        assert_eq!(
            (plan.at_ms, plan.length_ms, plan.start_b_ms),
            (180_000, UNMATCHED_MS, 14_000)
        );
        assert_eq!(
            plan.at_ms + plan.length_ms,
            170_000 + 16_000,
            "A leaves where the recipe says"
        );

        let same_tempo = Recipe {
            speed_b: 1.005,
            ..matched.clone()
        };
        assert_eq!(same_tempo.plan(200_000, 240_000).unwrap().length_ms, 16_000);
        let early_b = Recipe {
            start_b_ms: -12_000,
            ..matched
        };
        let plan = early_b.plan(200_000, 240_000).unwrap();
        assert_eq!((plan.start_b_ms, plan.lead_in_ms), (0, 2_000));
    }

    #[test]
    fn plays_only_items_at_their_own_speed() {
        assert!(plain_speed(0.));
        assert!(plain_speed(1.));
        assert!(plain_speed(1.004));
        assert!(!plain_speed(1.1));
        assert_eq!(item_speed(None), 1.);
        assert_eq!(item_speed(Some(" 1.25 ")), 1.25);
        assert_eq!(item_speed(Some("fast")), 1.);
        assert_eq!(item_speed(Some("-2")), 1.);
        let faster = Recipe {
            item_speeds: (1., 1.2),
            ..recipe()
        };
        assert!(!faster.plain_speeds());
        assert!(
            Recipe {
                item_speeds: (0., 0.),
                ..recipe()
            }
            .plain_speeds()
        );
    }

    #[test]
    fn spots_a_mixed_list() {
        let list = |pairs: &[(&str, &str)]| {
            pairs
                .iter()
                .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
                .collect::<HashMap<_, _>>()
        };
        assert!(mixed_list(&list(&[("mix", "true")])));
        assert!(!mixed_list(&list(&[("mix", "false")])));
        assert!(!mixed_list(&list(&[("decision_id", "x")])));
        assert!(mixed_item(&list(&[(RECIPE, "AAAA")])));
        assert!(mixed_item(&list(&[(REPORTING, MIXED_PLAYLIST)])));
        assert!(!mixed_item(&list(&[(REPORTING, "Other")])));
        assert!(!mixed_item(&list(&[("decision_id", "x")])));
    }
}
