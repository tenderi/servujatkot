//! Hero table (id, internal name, display name), from odota/dotaconstants.
//! Refresh `data/dota/heroes.json` when Valve adds heroes; see docs/DESIGN.md.

use std::collections::HashMap;
use std::sync::LazyLock;

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct Hero {
    pub id: u32,
    /// Internal name without the `npc_dota_hero_` prefix, e.g. `nevermore`.
    pub class: String,
    /// Display name, e.g. `Shadow Fiend`.
    pub name: String,
}

struct Table {
    by_id: HashMap<u32, Hero>,
    by_class: HashMap<String, u32>,
}

static TABLE: LazyLock<Table> = LazyLock::new(|| {
    let heroes: Vec<Hero> = serde_json::from_str(include_str!("../../data/dota/heroes.json"))
        .expect("data/dota/heroes.json is valid");
    Table {
        by_class: heroes.iter().map(|h| (h.class.clone(), h.id)).collect(),
        by_id: heroes.into_iter().map(|h| (h.id, h)).collect(),
    }
});

pub fn by_id(id: u32) -> Option<&'static Hero> {
    TABLE.by_id.get(&id)
}

/// Accepts the bare class (`antimage`) or the full `npc_dota_hero_antimage`.
pub fn by_class(class: &str) -> Option<&'static Hero> {
    let class = class.strip_prefix("npc_dota_hero_").unwrap_or(class);
    TABLE.by_class.get(class).and_then(|id| by_id(*id))
}

impl Hero {
    /// Full-body animated render (WebM with alpha) from Valve's CDN.
    pub fn render_video_url(&self) -> String {
        format!(
            "https://cdn.steamstatic.com/apps/dota2/videos/dota_react/heroes/renders/{}.webm",
            self.class
        )
    }

    /// Still of the same render (transparent PNG); poster and `?still` fallback.
    pub fn render_image_url(&self) -> String {
        format!(
            "https://cdn.steamstatic.com/apps/dota2/videos/dota_react/heroes/renders/{}.png",
            self.class
        )
    }

    /// Landscape portrait from Valve's CDN.
    pub fn portrait_url(&self) -> String {
        format!(
            "https://cdn.steamstatic.com/apps/dota2/images/dota_react/heroes/{}.png",
            self.class
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookups() {
        assert_eq!(by_id(1).unwrap().name, "Anti-Mage");
        assert_eq!(
            by_class("npc_dota_hero_nevermore").unwrap().name,
            "Shadow Fiend"
        );
        assert_eq!(by_class("nevermore").unwrap().id, 11);
        assert!(by_id(0).is_none());
    }
}
