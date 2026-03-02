use std::io::{self, Write as _};

use anyhow::{bail, Context};
use reqwest::header::{CACHE_CONTROL, COOKIE, PRAGMA, USER_AGENT};
use serde::Deserialize;
use tempfile::NamedTempFile;

/// Authentication method for Roblox API calls.
pub enum RobloxAuth {
    Cookie(String),
    ApiKey(String),
}

/// Resolve authentication: prefer `--opencloud` API key, fall back to system cookie.
pub fn resolve_auth(opencloud_key: Option<&str>) -> anyhow::Result<RobloxAuth> {
    if let Some(key) = opencloud_key {
        return Ok(RobloxAuth::ApiKey(key.to_string()));
    }

    let cookie = rbx_cookie::get_value().context(
        "No OpenCloud API key (--opencloud) or Roblox auth cookie found. \
         Pass --opencloud <KEY> or log into Roblox Studio.",
    )?;
    Ok(RobloxAuth::Cookie(cookie))
}

/// Try to resolve authentication, returning `None` if neither key nor cookie is available.
pub fn try_resolve_auth(opencloud_key: Option<&str>) -> Option<RobloxAuth> {
    if let Some(key) = opencloud_key {
        return Some(RobloxAuth::ApiKey(key.to_string()));
    }
    rbx_cookie::get_value().map(RobloxAuth::Cookie)
}

/// Download a place file from Roblox.
///
/// With API key: uses OpenCloud Asset Delivery API (`apis.roblox.com/asset-delivery-api`).
/// With cookie: uses legacy asset delivery (`assetdelivery.roblox.com`).
pub fn download_place(place_id: u64, auth: &RobloxAuth) -> anyhow::Result<NamedTempFile> {
    let bytes = match auth {
        RobloxAuth::ApiKey(key) => download_place_opencloud(place_id, key)?,
        RobloxAuth::Cookie(cookie) => download_place_cookie(place_id, cookie)?,
    };

    let mut temp_file = tempfile::Builder::new()
        .prefix("rojo-syncback-")
        .suffix(".rbxl")
        .tempfile()
        .context("Failed to create temporary file")?;

    io::copy(&mut bytes.as_slice(), &mut temp_file)?;
    temp_file.flush()?;

    log::debug!(
        "Downloaded {} bytes to {}",
        bytes.len(),
        temp_file.path().display()
    );

    Ok(temp_file)
}

fn download_place_opencloud(place_id: u64, api_key: &str) -> anyhow::Result<Vec<u8>> {
    let url = format!(
        "https://apis.roblox.com/asset-delivery-api/v1/assetId/{}",
        place_id
    );

    let client = reqwest::blocking::Client::builder()
        .gzip(true)
        .brotli(true)
        .deflate(true)
        .build()?;

    let response = client
        .get(&url)
        .header("x-api-key", api_key)
        .header("Accept-Encoding", "gzip")
        .send()?;

    let status = response.status();
    if !status.is_success() {
        bail!(
            "Failed to download place {} via OpenCloud: HTTP {} - {}",
            place_id,
            status,
            response.text().unwrap_or_default()
        );
    }

    let body: AssetDeliveryResponse = response.json().with_context(|| {
        format!(
            "Failed to parse OpenCloud asset delivery response for place {}",
            place_id
        )
    })?;

    let cdn_url = body.location.with_context(|| {
        format!(
            "OpenCloud asset delivery response for place {} has no location URL. \
             Ensure the API key has the 'legacy-asset:manage' scope.",
            place_id
        )
    })?;

    let cdn_response = client.get(&cdn_url).send()?;

    let cdn_status = cdn_response.status();
    if !cdn_status.is_success() {
        bail!(
            "Failed to download place {} from CDN: HTTP {} - {}",
            place_id,
            cdn_status,
            cdn_response.text().unwrap_or_default()
        );
    }

    Ok(cdn_response.bytes()?.to_vec())
}

fn download_place_cookie(place_id: u64, cookie: &str) -> anyhow::Result<Vec<u8>> {
    let url = format!("https://assetdelivery.roblox.com/v1/asset/?id={}", place_id);

    let client = reqwest::blocking::Client::builder()
        .gzip(true)
        .brotli(true)
        .deflate(true)
        .build()?;

    let response = client
        .get(&url)
        .header(COOKIE, format!(".ROBLOSECURITY={}", cookie))
        .header(CACHE_CONTROL, "no-cache, no-store, must-revalidate")
        .header(PRAGMA, "no-cache")
        .header("Expires", "0")
        .header(USER_AGENT, "Rojo")
        .send()?;

    let status = response.status();
    if !status.is_success() {
        bail!(
            "Failed to download place {}: HTTP {} - {}",
            place_id,
            status,
            response.text().unwrap_or_default()
        );
    }

    Ok(response.bytes()?.to_vec())
}

#[derive(Debug, Deserialize)]
struct AssetDeliveryResponse {
    location: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UniverseResponse {
    #[serde(rename = "universeId")]
    universe_id: u64,
}

#[derive(Debug, Deserialize)]
struct GamesResponse {
    data: Vec<GameData>,
}

#[derive(Debug, Deserialize)]
struct GameData {
    name: String,
}

/// Resolve a place ID to a universe ID.
///
/// Returns `None` if no auth is available. Works with both API key and cookie.
pub fn get_universe_id(place_id: u64, auth: &RobloxAuth) -> anyhow::Result<u64> {
    let url = format!(
        "https://apis.roblox.com/universes/v1/places/{}/universe",
        place_id
    );

    let client = reqwest::blocking::Client::new();
    let req = apply_auth(client.get(&url), auth);

    let response: UniverseResponse = req
        .send()
        .context("Failed to fetch universe ID from Roblox API")?
        .json()
        .context("Failed to parse universe ID response")?;

    Ok(response.universe_id)
}

/// Resolve a place ID to its experience name via the Roblox API.
///
/// With cookie auth, uses both the universe API and the games API.
/// With API key, the games.roblox.com endpoint may not support API key auth,
/// so this falls back gracefully.
pub fn fetch_experience_name(place_id: u64, auth: &RobloxAuth) -> anyhow::Result<Option<String>> {
    let client = reqwest::blocking::Client::new();

    let universe_url = format!(
        "https://apis.roblox.com/universes/v1/places/{}/universe",
        place_id
    );
    let req = apply_auth(client.get(&universe_url), auth);
    let universe: UniverseResponse = req
        .send()
        .context("Failed to reach Roblox universe API")?
        .json()
        .context("Failed to parse universe response")?;

    let games_url = format!(
        "https://games.roblox.com/v1/games?universeIds={}",
        universe.universe_id
    );

    // games.roblox.com may not support API key auth; try with auth, fall back to no-auth
    let req = apply_auth(client.get(&games_url), auth);
    let games_result: Result<GamesResponse, _> = req.send().and_then(|r| r.json());

    match games_result {
        Ok(games) => {
            if let Some(game) = games.data.into_iter().next() {
                println!("Experience: {}", game.name);
                return Ok(Some(game.name));
            }
        }
        Err(e) => {
            log::debug!(
                "Could not fetch game name (games API may not support this auth method): {}",
                e
            );
        }
    }

    // If API key was used and games endpoint failed, try without auth
    if matches!(auth, RobloxAuth::ApiKey(_)) {
        let req = client.get(&games_url);
        if let Ok(resp) = req.send() {
            if let Ok(games) = resp.json::<GamesResponse>() {
                if let Some(game) = games.data.into_iter().next() {
                    println!("Experience: {}", game.name);
                    return Ok(Some(game.name));
                }
            }
        }
    }

    Ok(None)
}

fn apply_auth(
    req: reqwest::blocking::RequestBuilder,
    auth: &RobloxAuth,
) -> reqwest::blocking::RequestBuilder {
    match auth {
        RobloxAuth::ApiKey(key) => req.header("x-api-key", key),
        RobloxAuth::Cookie(cookie) => req.header(COOKIE, format!(".ROBLOSECURITY={}", cookie)),
    }
}
