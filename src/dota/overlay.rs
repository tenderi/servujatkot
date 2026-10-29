//! HTML for the Dota 2 OBS overlays. Pages are rendered here in full; updates
//! arrive over SSE as re-rendered fragments that `static/overlay.js` swaps in.

use maud::{DOCTYPE, Markup, html};

use super::gsi::{Draft, Side, TeamDraft};
use super::heroes::Hero;

/// Captains Mode slot counts. The payload may carry fewer (or, for bans, none).
const PICKS: usize = 5;
const BANS: usize = 7;

pub struct Names<'a> {
    pub radiant: &'a str,
    pub dire: &'a str,
}

/// A full overlay page: transparent, sized for a 1920x1080 browser source.
pub fn page(title: &str, events: Option<&str>, body: Markup) -> Markup {
    html! {
        (DOCTYPE)
        html lang="en" {
            head {
                meta charset="utf-8";
                title { (title) }
                link rel="stylesheet" href="/static/overlay.css";
                @if events.is_some() { script src="/static/overlay.js" defer {} }
            }
            body.overlay {
                div #root data-events=[events] { (body) }
            }
        }
    }
}

pub fn draft(d: Option<&Draft>, names: &Names) -> Markup {
    let Some(d) = d else {
        return html! { div.draft.idle {} };
    };
    html! {
        div.draft {
            (team(Side::Radiant, names.radiant, &d.radiant, d, d.radiant_reserve))
            div.clock {
                @if let Some(side) = d.active {
                    div.phase.(side_class(side)) {
                        (side_label(side, names)) " " @if d.pick { "pick" } @else { "ban" }
                    }
                    div.time { (clock(d.time_remaining)) }
                }
            }
            (team(Side::Dire, names.dire, &d.dire, d, d.dire_reserve))
        }
    }
}

fn team(side: Side, name: &str, t: &TeamDraft, d: &Draft, reserve: i64) -> Markup {
    let active = d.active == Some(side);
    html! {
        section.team.(side_class(side)).active[active] {
            header {
                span.name { (name) }
                span.reserve title="Reserve time" { (clock(reserve)) }
            }
            div.picks { @for h in padded(&t.picks, PICKS) { (slot(h, "pick")) } }
            div.bans { @for h in padded(&t.bans, BANS) { (slot(h, "ban")) } }
        }
    }
}

fn slot(hero: Option<&Hero>, kind: &str) -> Markup {
    html! {
        @match hero {
            Some(h) => div.slot.(kind).filled title=(h.name) {
                img src=(h.portrait_url()) alt=(h.name);
                @if kind == "pick" { span.label { (h.name) } }
            },
            None => div.slot.(kind).empty {},
        }
    }
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

fn side_label<'a>(side: Side, names: &Names<'a>) -> &'a str {
    match side {
        Side::Radiant => names.radiant,
        Side::Dire => names.dire,
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

    #[test]
    fn renders_sample_draft() {
        let payload =
            serde_json::from_str(include_str!("../../data/dota/sample_draft.json")).unwrap();
        let d = parse_draft(&payload).unwrap();
        let html = draft(
            Some(&d),
            &Names {
                radiant: "Rad",
                dire: "Dire Co",
            },
        )
        .into_string();
        assert!(html.contains("Anti-Mage"));
        assert!(html.contains("Dire Co pick"));
        assert!(html.contains(">0:17<"));
        assert!(html.contains(">1:52<"), "radiant reserve");
        assert_eq!(html.matches("slot pick").count(), 10);
        assert_eq!(html.matches("slot ban").count(), 14);
        assert!(html.contains("team dire active"));
    }

    #[test]
    fn idle_without_draft() {
        let html = draft(
            None,
            &Names {
                radiant: "a",
                dire: "b",
            },
        )
        .into_string();
        assert_eq!(html, r#"<div class="draft idle"></div>"#);
    }
}
