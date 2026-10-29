//! Dota 2 Game State Integration: the cfg file that tells the client where to
//! POST, and parsing what it sends while spectating.
//!
//! The client reads `game/dota/cfg/gamestate_integration/gamestate_integration_*.cfg`
//! at startup and POSTs JSON to `uri` on every change (throttled). While
//! spectating it includes every player and the `draft` block; while playing,
//! only your own hero.

use serde_json::Value;

use super::heroes::{self, Hero};

pub const CFG_NAME: &str = "gamestate_integration_servujatkot.cfg";

/// Data blocks we ask for. `draft` is the one the draft overlay needs; the rest
/// are for the in-game overlays to come and cost nothing to request.
const COMPONENTS: &[&str] = &[
    "provider",
    "map",
    "player",
    "hero",
    "abilities",
    "items",
    "draft",
    "league",
    "buildings",
    "events",
    "roshan",
    "couriers",
    "neutralitems",
    "minimap",
    "wearables",
];

pub fn install_cfg(uri: &str, token: Option<&str>) -> String {
    let mut out = String::from("\"servujatkot\"\n{\n");
    out += &format!("    \"uri\"       \"{uri}\"\n");
    out += "    \"timeout\"   \"5.0\"\n    \"buffer\"    \"0.1\"\n    \"throttle\"  \"0.1\"\n    \"heartbeat\" \"10.0\"\n";
    if let Some(t) = token {
        out += &format!("    \"auth\"\n    {{\n        \"token\" \"{t}\"\n    }}\n");
    }
    out += "    \"data\"\n    {\n";
    for c in COMPONENTS {
        out += &format!("        \"{c}\" \"1\"\n");
    }
    out += "    }\n}\n";
    out
}

pub fn auth_token(payload: &Value) -> Option<&str> {
    payload.pointer("/auth/token").and_then(Value::as_str)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Radiant,
    Dire,
}

impl Side {
    fn from_gsi(team: i64) -> Option<Self> {
        match team {
            2 => Some(Side::Radiant),
            3 => Some(Side::Dire),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct TeamDraft {
    /// Indexed by the slot number in the payload; `None` = not picked yet.
    pub picks: Vec<Option<&'static Hero>>,
    pub bans: Vec<Option<&'static Hero>>,
}

#[derive(Debug, Clone, Default)]
pub struct Draft {
    pub active: Option<Side>,
    /// Whether the active team is picking (false = banning).
    pub pick: bool,
    pub time_remaining: i64,
    pub radiant_reserve: i64,
    pub dire_reserve: i64,
    pub radiant: TeamDraft,
    pub dire: TeamDraft,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct LeagueTeam {
    pub name: String,
    pub tag: String,
    pub series_wins: i64,
}

/// The `league` block, present when spectating a ticketed league match.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct League {
    pub name: String,
    /// e.g. `bo3`.
    pub series_type: String,
    pub radiant: LeagueTeam,
    pub dire: LeagueTeam,
}

impl League {
    /// Wins needed to take the series, from `boN`.
    pub fn wins_needed(&self) -> Option<i64> {
        let n: i64 = self.series_type.strip_prefix("bo")?.parse().ok()?;
        (n > 1).then_some(n / 2 + 1)
    }
}

pub fn parse_league(payload: &Value) -> Option<League> {
    let l = payload.get("league")?.as_object()?;
    let s = |v: Option<&Value>, k: &str| {
        v.and_then(|v| v.get(k))
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string()
    };
    let team = |side: &str| {
        let t = l.get(side);
        LeagueTeam {
            name: s(t, "name"),
            tag: s(t, "team_tag"),
            series_wins: t
                .and_then(|t| t.get("series_wins"))
                .and_then(Value::as_i64)
                .unwrap_or(0),
        }
    };
    let league = League {
        name: l
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string(),
        series_type: l
            .get("series_type")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        radiant: team("radiant"),
        dire: team("dire"),
    };
    (!league.name.is_empty() || !league.radiant.name.is_empty()).then_some(league)
}

/// `map.game_state`, e.g. `DOTA_GAMERULES_STATE_HERO_SELECTION`.
pub fn game_state(payload: &Value) -> Option<&str> {
    payload.pointer("/map/game_state").and_then(Value::as_str)
}

/// The `draft` block, or `None` if the payload has none (not spectating, or
/// not in a draft-capable mode). Bans are read the same way as picks
/// (`banN_id` / `banN_class`) in case the client sends them.
pub fn parse_draft(payload: &Value) -> Option<Draft> {
    let d = payload.get("draft")?.as_object()?;
    if d.is_empty() {
        return None;
    }
    let int = |k: &str| d.get(k).and_then(Value::as_i64).unwrap_or(0);
    Some(Draft {
        active: Side::from_gsi(int("activeteam")),
        pick: d.get("pick").and_then(Value::as_bool).unwrap_or(false),
        time_remaining: int("activeteam_time_remaining"),
        radiant_reserve: int("radiant_bonus_time"),
        dire_reserve: int("dire_bonus_time"),
        radiant: d.get("team2").map(parse_team).unwrap_or_default(),
        dire: d.get("team3").map(parse_team).unwrap_or_default(),
    })
}

fn parse_team(team: &Value) -> TeamDraft {
    TeamDraft {
        picks: slots(team, "pick"),
        bans: slots(team, "ban"),
    }
}

/// Collects `{prefix}N_id` / `{prefix}N_class` into a vector indexed by N.
fn slots(team: &Value, prefix: &str) -> Vec<Option<&'static Hero>> {
    let mut out: Vec<Option<&'static Hero>> = Vec::new();
    for n in 0.. {
        let id = team.get(format!("{prefix}{n}_id"));
        let class = team.get(format!("{prefix}{n}_class"));
        if id.is_none() && class.is_none() {
            break;
        }
        let hero = id
            .and_then(Value::as_u64)
            .and_then(|id| heroes::by_id(id as u32))
            .or_else(|| class.and_then(Value::as_str).and_then(heroes::by_class));
        out.push(hero);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const DRAFT: &str = include_str!("../../data/dota/sample_draft.json");

    #[test]
    fn parses_spectator_draft() {
        // A real capture (PARIVISION vs Team Yandex), trimmed; see docs/DESIGN.md.
        let payload: Value = serde_json::from_str(DRAFT).unwrap();
        assert_eq!(
            game_state(&payload),
            Some("DOTA_GAMERULES_STATE_HERO_SELECTION")
        );
        assert_eq!(auth_token(&payload), Some("test-token"));
        let d = parse_draft(&payload).unwrap();
        assert_eq!(d.active, Some(Side::Dire));
        assert!(d.pick);
        assert_eq!(d.time_remaining, 21);
        assert_eq!(d.radiant_reserve, 74);
        assert_eq!(d.radiant.picks.len(), 5);
        assert_eq!(d.radiant.picks[0].unwrap().name, "Earthshaker");
        assert_eq!(d.radiant.picks[1].unwrap().name, "Dark Willow");
        assert!(d.radiant.picks[2].is_none());
        assert_eq!(d.radiant.bans.len(), 7);
        assert_eq!(d.radiant.bans[0].unwrap().name, "Bounty Hunter");
        assert_eq!(d.dire.picks[0].unwrap().name, "Slark");
        assert_eq!(d.dire.bans[4].unwrap().name, "Techies");
        assert!(d.dire.bans[5].is_none());
    }

    #[test]
    fn parses_league() {
        let payload: Value = serde_json::from_str(DRAFT).unwrap();
        let l = parse_league(&payload).unwrap();
        assert_eq!(l.name, "PARI UNIVERSE by FISSURE");
        assert_eq!(l.radiant.name, "PARIVISION");
        assert_eq!(l.dire.tag, "TY");
        assert_eq!(l.wins_needed(), Some(2));
        assert!(parse_league(&serde_json::json!({"league": {}})).is_none());
    }

    #[test]
    fn hand_written_class_fallback() {
        // id 0 but a class: falls back to the class name.
        let payload = serde_json::json!({"draft": {"activeteam": 2, "team2": {"pick0_id": 0, "pick0_class": "nevermore"}}});
        let d = parse_draft(&payload).unwrap();
        assert_eq!(d.radiant.picks[0].unwrap().name, "Shadow Fiend");
    }

    #[test]
    fn no_draft_block() {
        assert!(parse_draft(&serde_json::json!({"map": {}})).is_none());
        assert!(parse_draft(&serde_json::json!({"draft": {}})).is_none());
    }

    #[test]
    fn cfg_has_uri_token_and_draft() {
        let cfg = install_cfg("http://127.0.0.1:3000/gsi/dota", Some("abc"));
        assert!(cfg.contains("\"uri\"       \"http://127.0.0.1:3000/gsi/dota\""));
        assert!(cfg.contains("\"token\" \"abc\""));
        assert!(cfg.contains("\"draft\" \"1\""));
        assert_eq!(cfg.matches('{').count(), cfg.matches('}').count());
    }
}
