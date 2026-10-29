//! HTML for the Dota 2 OBS overlays. Pages are rendered here in full; updates
//! arrive over SSE as re-rendered fragments that `static/overlay.js` merges in.

use maud::{DOCTYPE, Markup, html};

use super::gsi::{Draft, Side, TeamDraft};
use super::heroes::Hero;
use super::players::PickedBy;

/// Captains Mode slot counts. The payload may carry fewer.
const PICKS: usize = 5;
const BANS: usize = 7;

/// Seconds left at which the fullscreen timer turns urgent.
const URGENT: i64 = 5;

/// Games on a hero before its win rate is worth showing.
const MIN_GAMES_FOR_RATE: u32 = 5;

pub struct TeamInfo {
    pub name: String,
    pub tag: String,
    pub wins: i64,
    /// Who plays each pick slot, known or guessed; may be shorter than the picks.
    pub picked_by: Vec<Option<PickedBy>>,
}

impl TeamInfo {
    fn by(&self, slot: usize) -> Option<&PickedBy> {
        self.picked_by.get(slot).and_then(Option::as_ref)
    }
}

/// Everything about the match that isn't the draft itself.
pub struct MatchInfo {
    pub radiant: TeamInfo,
    pub dire: TeamInfo,
    pub event: Option<String>,
    /// Wins needed to take the series (2 for a Bo3); `None` = no series.
    pub wins_needed: Option<i64>,
}

impl MatchInfo {
    fn team(&self, side: Side) -> &TeamInfo {
        match side {
            Side::Radiant => &self.radiant,
            Side::Dire => &self.dire,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum View {
    /// The lower-third bar over gameplay.
    Bar,
    /// A whole 1920x1080 scene. `still` swaps the animated hero renders for
    /// PNGs, for slower OBS machines.
    Full { still: bool },
}

impl View {
    pub fn render(self, d: Option<&Draft>, m: &MatchInfo) -> Markup {
        match self {
            View::Bar => bar(d, m),
            View::Full { still } => full(d, m, still),
        }
    }

    fn styles(self) -> &'static [&'static str] {
        match self {
            View::Bar => &["/static/overlay.css"],
            View::Full { .. } => &[
                "https://fonts.googleapis.com/css2?family=Barlow+Condensed:wght@500;700;800;900&display=swap",
                "/static/overlay.css",
                "/static/draft-full.css",
            ],
        }
    }
}

/// A full overlay page sized for a 1920x1080 browser source.
pub fn page(title: &str, view: View, events: Option<&str>, body: Markup) -> Markup {
    let body_class = match view {
        View::Bar => "overlay",
        View::Full { .. } => "overlay fullscreen",
    };
    html! {
        (DOCTYPE)
        html lang="en" {
            head {
                meta charset="utf-8";
                title { (title) }
                @for href in view.styles() { link rel="stylesheet" href=(href); }
                @if events.is_some() { script src="/static/overlay.js" defer {} }
            }
            body class=(body_class) {
                div #root data-events=[events] { (body) }
            }
        }
    }
}

// ---------------------------------------------------------------- bar

fn bar(d: Option<&Draft>, m: &MatchInfo) -> Markup {
    let Some(d) = d else {
        return html! { div.draft.idle {} };
    };
    html! {
        div.draft {
            (bar_team(Side::Radiant, m, &d.radiant, d, d.radiant_reserve))
            div.clock {
                @if let Some(side) = d.active {
                    div.phase.(side_class(side)) {
                        (m.team(side).name) " " (phase_word(d).to_lowercase())
                    }
                    div.time { (clock(d.time_remaining)) }
                }
            }
            (bar_team(Side::Dire, m, &d.dire, d, d.dire_reserve))
        }
    }
}

fn bar_team(side: Side, m: &MatchInfo, t: &TeamDraft, d: &Draft, reserve: i64) -> Markup {
    let active = d.active == Some(side);
    html! {
        section.team.(side_class(side)).active[active] {
            header {
                span.name { (m.team(side).name) }
                span.reserve title="Reserve time" { (clock(reserve)) }
            }
            div.picks {
                @for (i, h) in padded(&t.picks, PICKS).into_iter().enumerate() {
                    (bar_slot(h, "pick", m.team(side).by(i)))
                }
            }
            div.bans { @for h in padded(&t.bans, BANS) { (bar_slot(h, "ban", None)) } }
        }
    }
}

fn bar_slot(hero: Option<&Hero>, kind: &str, by: Option<&PickedBy>) -> Markup {
    html! {
        @match hero {
            Some(h) => div.slot.(kind).filled title=(h.name) {
                img src=(h.portrait_url()) alt=(h.name);
                @if let Some(b) = by {
                    span.player.guess[b.record.is_some()] { (b.name) }
                }
                @if kind == "pick" { span.label { (h.name) } }
            },
            None => div.slot.(kind).empty {},
        }
    }
}

// ---------------------------------------------------------- fullscreen

fn full(d: Option<&Draft>, m: &MatchInfo, still: bool) -> Markup {
    let active = d.and_then(|d| d.active);
    let urgent = d.is_some_and(|d| d.active.is_some() && d.time_remaining <= URGENT);
    html! {
        div.full.idle[d.is_none()].urgent[urgent] data-active=[active.map(side_class)] {
            div.backdrop {}
            header.event {
                @if let Some(e) = &m.event { span.event-name { (e) } }
                @if let Some(n) = m.wins_needed {
                    span.series { "Best of " (2 * n - 1) }
                }
            }
            (full_side(Side::Radiant, m, d, still))
            div.center {
                @match d {
                    Some(d) => {
                        @if let Some(side) = d.active {
                            div.phase-word aria-hidden="true" { (phase_word(d)) }
                            div.on-clock.(side_class(side)) {
                                span.who { (m.team(side).name) }
                                span.what { (phase_word(d)) }
                            }
                            div.timer { (clock(d.time_remaining)) }
                            div.step { (step_label(d)) }
                        }
                    }
                    None => {
                        div.versus { "VS" }
                        div.step { "Draft starts soon" }
                    }
                }
            }
            (full_side(Side::Dire, m, d, still))
        }
    }
}

fn full_side(side: Side, m: &MatchInfo, d: Option<&Draft>, still: bool) -> Markup {
    let info = m.team(side);
    let empty = TeamDraft::default();
    let (t, reserve) = match (d, side) {
        (Some(d), Side::Radiant) => (&d.radiant, Some(d.radiant_reserve)),
        (Some(d), Side::Dire) => (&d.dire, Some(d.dire_reserve)),
        (None, _) => (&empty, None),
    };
    let active = d.is_some_and(|d| d.active == Some(side));
    // The slot about to be filled, if this side is on the clock.
    let next = d.filter(|_| active).and_then(|d| {
        let (slots, n) = if d.pick {
            (&t.picks, PICKS)
        } else {
            (&t.bans, BANS)
        };
        padded(slots, n)
            .iter()
            .position(Option::is_none)
            .map(|i| (d.pick, i))
    });
    html! {
        section.side.(side_class(side)).active[active] {
            div.team-head {
                @if !info.tag.is_empty() { span.tag { (info.tag) } }
                span.team-name.long[info.name.chars().count() > 13] { (info.name) }
                @if let Some(n) = m.wins_needed {
                    span.pips title="Series wins" {
                        @for i in 0..n { i.pip.won[i < info.wins] {} }
                    }
                }
            }
            div.cards {
                @for (i, h) in padded(&t.picks, PICKS).into_iter().enumerate() {
                    (card(h, i, next == Some((true, i)), still, info.by(i)))
                }
            }
            div.ban-row {
                span.ban-label { "Bans" }
                @for (i, h) in padded(&t.bans, BANS).into_iter().enumerate() {
                    (ban(h, next == Some((false, i))))
                }
            }
            @if let Some(r) = reserve {
                div.reserve { span { "Reserve" } b { (clock(r)) } }
            }
        }
    }
}

fn card(hero: Option<&Hero>, i: usize, next: bool, still: bool, by: Option<&PickedBy>) -> Markup {
    html! {
        @match hero {
            Some(h) => div.card.filled data-hero=(h.class) {
                div.art {
                    @if still {
                        img src=(h.render_image_url()) alt="";
                    } @else {
                        video src=(h.render_video_url()) poster=(h.render_image_url())
                            autoplay muted loop playsinline {}
                    }
                }
                @if let Some(b) = by { (played_by(b)) }
                div.hero-name { (h.name) }
            },
            None => div.card.empty.next[next] {
                div.slot-no { (i + 1) }
            },
        }
    }
}

/// The callout naming who plays a pick: certain once the player has taken
/// the hero, otherwise the most likely player and their record on it.
fn played_by(b: &PickedBy) -> Markup {
    html! {
        div.played-by.guess[b.record.is_some()] {
            @match b.record {
                Some(r) => {
                    span.by-hint { "Likely" }
                    span.by-name { (b.name) }
                    span.by-stat {
                        (r.games) @if r.games == 1 { " game" } @else { " games" }
                        // A win rate over a handful of games says nothing.
                        @if r.games >= MIN_GAMES_FOR_RATE {
                            " · " (100 * r.wins / r.games) "%"
                        }
                    }
                }
                None => span.by-name { (b.name) },
            }
        }
    }
}

fn ban(hero: Option<&Hero>, next: bool) -> Markup {
    html! {
        @match hero {
            Some(h) => div.ban.filled data-hero=(h.class) title=(h.name) {
                img src=(h.portrait_url()) alt=(h.name);
            },
            None => div.ban.empty.next[next] {},
        }
    }
}

/// "Pick 6 of 10" style progress through the draft.
fn step_label(d: &Draft) -> String {
    let filled = |f: fn(&TeamDraft) -> &Vec<Option<&'static Hero>>| {
        [&d.radiant, &d.dire]
            .iter()
            .map(|t| f(t).iter().flatten().count())
            .sum::<usize>()
    };
    if d.pick {
        format!("Pick {} of {}", filled(|t| &t.picks) + 1, 2 * PICKS)
    } else {
        format!("Ban {} of {}", filled(|t| &t.bans) + 1, 2 * BANS)
    }
}

// ------------------------------------------------------------- shared

fn phase_word(d: &Draft) -> &'static str {
    if d.pick { "Pick" } else { "Ban" }
}

fn padded(slots: &[Option<&'static Hero>], n: usize) -> Vec<Option<&'static Hero>> {
    let mut v = slots.to_vec();
    if v.len() < n {
        v.resize(n, None);
    }
    v
}

fn side_class(side: Side) -> &'static str {
    match side {
        Side::Radiant => "radiant",
        Side::Dire => "dire",
    }
}

fn clock(secs: i64) -> String {
    let s = secs.max(0);
    format!("{}:{:02}", s / 60, s % 60)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dota::gsi::parse_draft;

    fn info() -> MatchInfo {
        MatchInfo {
            radiant: TeamInfo {
                name: "Rad".into(),
                tag: "R".into(),
                wins: 1,
                picked_by: vec![
                    None,
                    Some(PickedBy {
                        name: "Carry Guy".into(),
                        record: Some(super::super::players::Record {
                            games: 26,
                            wins: 21,
                        }),
                    }),
                ],
            },
            dire: TeamInfo {
                name: "Dire Co".into(),
                tag: "".into(),
                wins: 0,
                picked_by: vec![Some(PickedBy {
                    name: "Known".into(),
                    record: None,
                })],
            },
            event: Some("Some Cup".into()),
            wins_needed: Some(2),
        }
    }

    fn sample() -> Draft {
        let payload =
            serde_json::from_str(include_str!("../../data/dota/sample_draft.json")).unwrap();
        parse_draft(&payload).unwrap()
    }

    #[test]
    fn bar_renders_sample() {
        let html = View::Bar.render(Some(&sample()), &info()).into_string();
        assert!(html.contains("Earthshaker"));
        assert!(html.contains("Dire Co pick"));
        assert!(html.contains(">0:21<"));
        assert!(html.contains(">1:14<"), "radiant reserve");
        assert_eq!(html.matches("slot pick").count(), 10);
        assert_eq!(html.matches("slot ban").count(), 14);
        assert!(html.contains("team dire active"));
    }

    #[test]
    fn bar_idle_without_draft() {
        let html = View::Bar.render(None, &info()).into_string();
        assert_eq!(html, r#"<div class="draft idle"></div>"#);
    }

    #[test]
    fn full_renders_sample() {
        let html = View::Full { still: false }
            .render(Some(&sample()), &info())
            .into_string();
        assert!(html.contains(r#"data-active="dire""#));
        assert!(html.contains("renders/earthshaker.webm"));
        assert_eq!(html.matches("card filled").count(), 3);
        assert_eq!(html.matches("card empty").count(), 7);
        // Dire is picking; its first empty pick slot (index 1) pulses, nothing else.
        assert_eq!(html.matches(" next").count(), 1);
        assert!(html.contains("Pick 4 of 10"));
        assert!(html.contains("Best of 3"));
        assert_eq!(html.matches("pip won").count(), 1);
        assert!(!html.contains("urgent"));
        assert!(html.contains("Carry Guy"));
        assert!(html.contains("26 games · 80%"));
        assert_eq!(html.matches("played-by guess").count(), 1);
        assert!(html.contains(r#"<div class="played-by"><span class="by-name">Known</span>"#));

        let still = View::Full { still: true }
            .render(Some(&sample()), &info())
            .into_string();
        assert!(!still.contains("<video"));
        assert!(still.contains("renders/earthshaker.png"));
    }

    #[test]
    fn full_turns_urgent_and_idles() {
        let mut d = sample();
        d.time_remaining = 4;
        let html = View::Full { still: true }
            .render(Some(&d), &info())
            .into_string();
        assert!(html.contains("full urgent"));

        let idle = View::Full { still: true }
            .render(None, &info())
            .into_string();
        assert!(idle.contains("full idle"));
        assert!(idle.contains("Draft starts soon"));
        assert!(idle.contains("Dire Co"));
    }
}
