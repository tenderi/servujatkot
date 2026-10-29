//! Who is likely to play each picked hero.
//!
//! While spectating, GSI's `player` block names all ten players with their
//! account ids, even during the draft. The `hero` block says which hero each
//! has, but only once they take one from the pool after the draft. Until then
//! we guess from each player's public match history on OpenDota.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::Value;

use super::gsi::TeamDraft;
use super::heroes::{self, Hero};

#[derive(Debug, Clone)]
pub struct Player {
    pub account_id: u64,
    pub name: String,
    /// The hero this player has taken; `None` during the draft.
    pub hero: Option<&'static Hero>,
}

#[derive(Debug, Clone, Default)]
pub struct Roster {
    pub radiant: Vec<Player>,
    pub dire: Vec<Player>,
}

impl Roster {
    pub fn all(&self) -> impl Iterator<Item = &Player> {
        self.radiant.iter().chain(&self.dire)
    }
}

/// `player.team2.player0..4` / `player.team3.player5..9`, with `hero.<team>.<player>.id`.
pub fn parse_roster(payload: &Value) -> Option<Roster> {
    let team = |t: &str| -> Vec<Player> {
        let Some(players) = payload
            .pointer(&format!("/player/{t}"))
            .and_then(Value::as_object)
        else {
            return Vec::new();
        };
        let mut keys: Vec<&String> = players.keys().collect();
        keys.sort_by_key(|k| {
            k.trim_start_matches("player")
                .parse::<u32>()
                .unwrap_or(u32::MAX)
        });
        keys.into_iter()
            .filter_map(|k| {
                let p = &players[k];
                // Strings in practice; numbers accepted in case that changes.
                let account_id = match p.get("accountid")? {
                    Value::String(s) => s.parse().ok()?,
                    v => v.as_u64()?,
                };
                let hero = payload
                    .pointer(&format!("/hero/{t}/{k}/id"))
                    .and_then(Value::as_u64)
                    .filter(|&id| id > 0)
                    .and_then(|id| heroes::by_id(id as u32));
                Some(Player {
                    account_id,
                    name: p
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    hero,
                })
            })
            .collect()
    };
    let roster = Roster {
        radiant: team("team2"),
        dire: team("team3"),
    };
    (!roster.radiant.is_empty() || !roster.dire.is_empty()).then_some(roster)
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Record {
    pub games: u32,
    pub wins: u32,
}

/// One player's games per hero id. Empty = no public history.
pub type HeroStats = HashMap<u32, Record>;

#[derive(Deserialize)]
struct Row {
    hero_id: u32,
    games: u32,
    win: u32,
}

/// Parses OpenDota's `/players/{id}/heroes` response.
pub fn parse_hero_stats(json: &str) -> Result<HeroStats> {
    let rows: Vec<Row> = serde_json::from_str(json).context("parsing OpenDota hero stats")?;
    Ok(rows
        .into_iter()
        .filter(|r| r.games > 0)
        .map(|r| {
            (
                r.hero_id,
                Record {
                    games: r.games,
                    wins: r.win,
                },
            )
        })
        .collect())
}

/// Fetches player hero stats from OpenDota. No key needed; the free tier
/// allows 60 calls a minute, and a match costs ten.
#[derive(Clone)]
pub struct OpenDota {
    client: reqwest::Client,
    /// Only games from the last this many days; 0 = all time.
    pub days: u32,
}

impl OpenDota {
    pub fn new(days: u32) -> Result<Self> {
        let client = reqwest::Client::builder()
            // OpenDota's CDN rejects requests without a user agent.
            .user_agent(concat!("servujatkot/", env!("CARGO_PKG_VERSION")))
            .timeout(std::time::Duration::from_secs(15))
            .build()?;
        Ok(OpenDota { client, days })
    }

    pub async fn hero_stats(&self, account_id: u64) -> Result<HeroStats> {
        let mut url = format!("https://api.opendota.com/api/players/{account_id}/heroes");
        if self.days > 0 {
            url += &format!("?date={}", self.days);
        }
        let body = self
            .client
            .get(&url)
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;
        parse_hero_stats(&body)
    }
}

/// Who plays a picked hero, for the overlay.
#[derive(Debug, Clone, PartialEq)]
pub struct PickedBy {
    pub name: String,
    /// The player's games on the hero; `None` = confirmed by the game itself.
    pub record: Option<Record>,
}

/// For each pick slot of `team`, who plays it: the player who has taken the
/// hero if the game says so, else the guess. Guesses give each player at most
/// one hero and maximise the total games played on the picked heroes, so two
/// heroes one player favours don't both go to them. A hero nobody has played
/// gets no name.
pub fn picked_by(
    team: &[Player],
    draft: &TeamDraft,
    stats: &dyn Fn(u64) -> Option<Arc<HeroStats>>,
) -> Vec<Option<PickedBy>> {
    let mut out: Vec<Option<PickedBy>> = vec![None; draft.picks.len()];
    let mut free_players: Vec<&Player> = Vec::new();
    for p in team {
        let slot = p.hero.and_then(|h| {
            draft
                .picks
                .iter()
                .position(|s| s.is_some_and(|s| s.id == h.id))
        });
        match slot {
            Some(i) => {
                out[i] = Some(PickedBy {
                    name: p.name.clone(),
                    record: None,
                })
            }
            None => free_players.push(p),
        }
    }
    let open: Vec<(usize, u32)> = draft
        .picks
        .iter()
        .enumerate()
        .filter(|(i, _)| out[*i].is_none())
        .filter_map(|(i, h)| h.map(|h| (i, h.id)))
        .collect();
    let records: Vec<HeroStats> = free_players
        .iter()
        .map(|p| {
            stats(p.account_id)
                .map(|s| (*s).clone())
                .unwrap_or_default()
        })
        .collect();
    let games = |player: usize, hero: u32| records[player].get(&hero).copied();

    // At most 5 picks and 5 players: trying every assignment is instant.
    fn search(
        open: &[(usize, u32)],
        used: &mut Vec<bool>,
        games: &dyn Fn(usize, u32) -> Option<Record>,
    ) -> (u32, Vec<Option<usize>>) {
        let Some((&(_, hero), rest)) = open.split_first() else {
            return (0, Vec::new());
        };
        let (mut best, mut tail) = search(rest, used, games);
        let mut pick = None;
        for p in 0..used.len() {
            let Some(r) = games(p, hero).filter(|_| !used[p]) else {
                continue;
            };
            used[p] = true;
            let (score, t) = search(rest, used, games);
            used[p] = false;
            if score + r.games > best {
                best = score + r.games;
                tail = t;
                pick = Some(p);
            }
        }
        tail.insert(0, pick);
        (best, tail)
    }
    let mut used = vec![false; free_players.len()];
    let (_, choice) = search(&open, &mut used, &games);
    for (&(slot, hero), p) in open.iter().zip(choice) {
        if let Some(p) = p {
            out[slot] = Some(PickedBy {
                name: free_players[p].name.clone(),
                record: games(p, hero),
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dota::gsi::parse_draft;

    const DRAFT: &str = include_str!("../../data/dota/sample_draft.json");
    const STATS: &str = include_str!("../../data/dota/sample_player_heroes.json");

    fn sample_stats() -> HashMap<u64, Arc<HeroStats>> {
        let all: HashMap<String, Value> = serde_json::from_str(STATS).unwrap();
        all.into_iter()
            .map(|(id, rows)| {
                (
                    id.parse().unwrap(),
                    Arc::new(parse_hero_stats(&rows.to_string()).unwrap()),
                )
            })
            .collect()
    }

    #[test]
    fn parses_roster_from_spectator_payload() {
        let payload: Value = serde_json::from_str(DRAFT).unwrap();
        let r = parse_roster(&payload).unwrap();
        assert_eq!(r.radiant.len(), 5);
        assert_eq!(r.dire.len(), 5);
        assert_eq!(r.radiant[0].name, "Satanic");
        assert_eq!(r.radiant[0].account_id, 1044002267);
        assert_eq!(r.dire[4].name, "Saksa");
        assert!(
            r.all().all(|p| p.hero.is_none()),
            "no heroes during the draft"
        );
        assert!(parse_roster(&serde_json::json!({"map": {}})).is_none());
    }

    #[test]
    fn guesses_from_history() {
        let payload: Value = serde_json::from_str(DRAFT).unwrap();
        let r = parse_roster(&payload).unwrap();
        let d = parse_draft(&payload).unwrap();
        let stats = sample_stats();
        let lookup = |id: u64| stats.get(&id).cloned();

        let rad = picked_by(&r.radiant, &d.radiant, &lookup);
        assert_eq!(rad.len(), 5);
        assert_eq!(rad[0].as_ref().unwrap().name, "No[o]ne-", "Earthshaker");
        assert_eq!(rad[1].as_ref().unwrap().name, "9Class", "Dark Willow");
        assert!(rad[1].as_ref().unwrap().record.unwrap().games > 0);
        assert!(rad[2].is_none(), "not picked yet");

        let dire = picked_by(&r.dire, &d.dire, &lookup);
        assert_eq!(dire[0].as_ref().unwrap().name, "CHIRA_JUNIOR", "Slark");
    }

    #[test]
    fn one_hero_per_player_and_confirmed_wins() {
        let hero = |n| heroes::by_class(n).unwrap();
        let (am, cm) = (hero("antimage"), hero("crystal_maiden"));
        let player = |id, name: &str| Player {
            account_id: id,
            name: name.into(),
            hero: None,
        };
        let team = vec![player(1, "A"), player(2, "B")];
        let draft = TeamDraft {
            picks: vec![Some(am), Some(cm)],
            bans: vec![],
        };
        let rec = |games| Record { games, wins: 0 };
        let stats: HashMap<u64, Arc<HeroStats>> = HashMap::from([
            // A plays both most, but B only plays Crystal Maiden.
            (
                1,
                Arc::new(HashMap::from([(am.id, rec(10)), (cm.id, rec(12))])),
            ),
            (2, Arc::new(HashMap::from([(cm.id, rec(3))]))),
        ]);
        let lookup = |id: u64| stats.get(&id).cloned();
        let got = picked_by(&team, &draft, &lookup);
        assert_eq!(got[0].as_ref().unwrap().name, "A");
        assert_eq!(got[1].as_ref().unwrap().name, "B");

        // The game says B took Anti-Mage: that beats any guess.
        let mut team = team;
        team[1].hero = Some(am);
        let got = picked_by(&team, &draft, &lookup);
        assert_eq!(
            got[0],
            Some(PickedBy {
                name: "B".into(),
                record: None
            })
        );
        assert_eq!(got[1].as_ref().unwrap().name, "A");
    }

    #[test]
    fn nobody_for_an_unplayed_hero() {
        let team = vec![Player {
            account_id: 1,
            name: "A".into(),
            hero: None,
        }];
        let draft = TeamDraft {
            picks: vec![Some(heroes::by_class("techies").unwrap())],
            bans: vec![],
        };
        let none = |_| None;
        assert_eq!(picked_by(&team, &draft, &none), vec![None]);
    }
}
