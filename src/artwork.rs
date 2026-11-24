//! Library artwork for new shortcuts, from SteamGridDB (needs the user's own free API key:
//! https://www.steamgriddb.com/profile/preferences/api).
//!
//! Files go in `userdata/<account>/config/grid/`, named after the shortcut's app ID the way Steam
//! expects. Existing artwork is never replaced, so anything the user set by hand stays.

use std::path::Path;

use anyhow::{Context, Result};
use serde_json::Value;

use crate::net;

const API: &str = "https://www.steamgriddb.com/api/v2";

/// (file name suffix after the app ID, API path after `/game/<id>`, what it is).
const KINDS: &[(&str, &str, &str)] = &[
    ("p", "grids/game/{}?dimensions=600x900", "cover"),
    ("", "grids/game/{}?dimensions=920x430,460x215", "wide cover"),
    ("_hero", "heroes/game/{}", "background"),
    ("_logo", "logos/game/{}", "logo"),
];

fn auth(key: &str) -> Vec<String> {
    vec![format!("Authorization: Bearer {}", key.trim())]
}

/// SteamGridDB's ID for the best match on `name`.
fn find_game(name: &str, key: &str) -> Result<Option<u64>> {
    let reply = net::get_json(
        &format!("{API}/search/autocomplete/{}", net::encode(name)),
        &auth(key),
    )?;
    Ok(reply["data"].get(0).and_then(|g| g["id"].as_u64()))
}

fn first_url(reply: &Value) -> Option<String> {
    reply["data"].get(0)?["url"].as_str().map(str::to_string)
}

fn has_file(grid: &Path, stem: &str) -> bool {
    ["png", "jpg", "jpeg", "webp"]
        .iter()
        .any(|ext| grid.join(format!("{stem}.{ext}")).exists())
}

/// Downloads whatever artwork is missing for one shortcut. Returns a line for the user.
pub fn fetch(name: &str, appid: u32, grid: &Path, key: &str) -> Result<String> {
    std::fs::create_dir_all(grid).with_context(|| format!("creating {}", grid.display()))?;
    let missing: Vec<_> = KINDS
        .iter()
        .filter(|(suffix, ..)| !has_file(grid, &format!("{appid}{suffix}")))
        .collect();
    if missing.is_empty() {
        return Ok(format!("{name}: already has artwork"));
    }
    let Some(id) = find_game(name, key)? else {
        return Ok(format!("{name}: no artwork found on SteamGridDB"));
    };
    let mut got = Vec::new();
    for (suffix, path, what) in missing {
        let reply = net::get_json(
            &format!("{API}/{}", path.replace("{}", &id.to_string())),
            &auth(key),
        )?;
        let Some(url) = first_url(&reply) else {
            continue;
        };
        let ext = url
            .rsplit('.')
            .next()
            .filter(|e| e.len() <= 4)
            .unwrap_or("png")
            .to_ascii_lowercase();
        // Image hosts don't need (and shouldn't get) the API key.
        net::download(&url, &grid.join(format!("{appid}{suffix}.{ext}")), &[])?;
        got.push(*what);
    }
    Ok(if got.is_empty() {
        format!("{name}: no artwork found on SteamGridDB")
    } else {
        format!("{name}: downloaded {}", got.join(", "))
    })
}
