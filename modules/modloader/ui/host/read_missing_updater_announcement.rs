use serde_json::{Value, json};
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

const CHECK_INTERVAL_SECONDS: u64 = 24 * 60 * 60;
const IN_FLIGHT_RETRY_SECONDS: u64 = 30;
const ANNOUNCEMENT_STATE_KEY: &str = "missingUpdaterAnnouncement";

pub(crate) fn read_cached(root: &Path) -> Value {
    shroudforge_loader_config::config::read_state_section(root, "updates")
        .ok()
        .flatten()
        .and_then(|updates| updates.get(ANNOUNCEMENT_STATE_KEY).cloned())
        .map(public_announcement)
        .unwrap_or_else(|| json!({"updateAvailable": false}))
}

pub(crate) fn start_refresh(root: &Path, central: &Value, cached: Arc<Mutex<Value>>) {
    if !module_is_explicitly_disabled(central) || !announcement_check_is_due(root, unix_seconds()) {
        return;
    }

    let root = root.to_path_buf();
    thread::spawn(move || refresh(&root, &cached));
}

fn refresh(root: &Path, cached: &Arc<Mutex<Value>>) {
    let Ok(config) = shroudforge_loader_config::config::read_loader(root) else {
        return;
    };
    if !eligible_for_release_news(&config) {
        return;
    }

    let now = unix_seconds();
    match claim_daily_check(root, now) {
        Ok(true) => {}
        Ok(false) => return,
        Err(error) => {
            let _ = shroudforge_logging::append(
                root,
                'W',
                "modloader",
                &format!("Could not reserve the missing-updater release check: {error}"),
            );
            return;
        }
    }

    let installed = installed_release(root);
    let include_prereleases = config
        .pointer("/modules/updates/system/includePrereleases")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let result = shroudforge_release_feed::fetch_latest(
        shroudforge_release_feed::GITHUB_API_BASE_URL,
        shroudforge_release_feed::SHROUDFORGE_PROJECT_ID,
        &installed.0,
        include_prereleases,
        false,
    );
    match result {
        Ok(release) => {
            let Ok(current_config) = shroudforge_loader_config::config::read_loader(root) else {
                release_daily_check(root, now);
                return;
            };
            if !eligible_for_release_news(&current_config) {
                release_daily_check(root, now);
                return;
            }
            let update_available = release.as_ref().is_some_and(|release| {
                shroudforge_release_feed::is_update_available(
                    &installed.0,
                    installed.1,
                    &release.base_version,
                    release.build,
                )
            });
            let announcement = release
                .filter(|_| update_available)
                .map(|release| {
                    json!({
                        "lastAttemptAt": now,
                        "checkedAt": now,
                        "updateAvailable": true,
                        "version": release.version,
                        "releaseUrl": release.release_url,
                        "changelog": release_changelog(&release.body)
                    })
                })
                .unwrap_or_else(|| {
                    json!({
                        "lastAttemptAt": now,
                        "checkedAt": now,
                        "updateAvailable": false,
                        "version": null,
                        "releaseUrl": null,
                        "changelog": []
                    })
                });
            if let Err(error) = save_announcement(root, announcement.clone()) {
                let _ = shroudforge_logging::append(
                    root,
                    'W',
                    "modloader",
                    &format!("Could not save the missing-updater release result: {error}"),
                );
            }
            if let Ok(mut value) = cached.lock() {
                *value = public_announcement(announcement);
            }
        }
        Err(error) => {
            let _ = shroudforge_logging::append(
                root,
                'W',
                "modloader",
                &format!("Could not refresh ShroudForge release news: {error}"),
            );
            release_daily_check(root, now);
        }
    }
}

fn claim_daily_check(root: &Path, now: u64) -> Result<bool, String> {
    let mut should_check = false;
    shroudforge_loader_config::config::update_state_section(root, "updates", |existing| {
        let mut updates = existing
            .cloned()
            .filter(Value::is_object)
            .unwrap_or_else(|| json!({}));
        let mut announcement = updates
            .get(ANNOUNCEMENT_STATE_KEY)
            .cloned()
            .filter(Value::is_object)
            .unwrap_or_else(|| empty_announcement(now));
        if announcement_check_due(&announcement, now) {
            announcement["lastAttemptAt"] = json!(now);
            should_check = true;
        }
        updates[ANNOUNCEMENT_STATE_KEY] = announcement;
        Ok(updates)
    })?;
    Ok(should_check)
}

fn announcement_check_is_due(root: &Path, now: u64) -> bool {
    let announcement = shroudforge_loader_config::config::read_state_section(root, "updates")
        .ok()
        .flatten()
        .and_then(|updates| updates.get(ANNOUNCEMENT_STATE_KEY).cloned())
        .filter(Value::is_object)
        .unwrap_or_else(|| empty_announcement(0));
    announcement_check_due(&announcement, now)
}

fn announcement_check_due(announcement: &Value, now: u64) -> bool {
    if let Some(checked_at) = announcement["checkedAt"].as_u64() {
        return now.saturating_sub(checked_at) >= CHECK_INTERVAL_SECONDS;
    }
    announcement["lastAttemptAt"]
        .as_u64()
        .is_none_or(|last| now.saturating_sub(last) >= IN_FLIGHT_RETRY_SECONDS)
}

fn release_daily_check(root: &Path, claimed_at: u64) {
    let _ = shroudforge_loader_config::config::update_state_section(root, "updates", |existing| {
        let mut updates = existing
            .cloned()
            .filter(Value::is_object)
            .unwrap_or_else(|| json!({}));
        if updates[ANNOUNCEMENT_STATE_KEY]["lastAttemptAt"].as_u64() == Some(claimed_at) {
            updates[ANNOUNCEMENT_STATE_KEY]["lastAttemptAt"] = json!(0);
        }
        Ok(updates)
    });
}

fn save_announcement(root: &Path, announcement: Value) -> Result<(), String> {
    shroudforge_loader_config::config::update_state_section(root, "updates", |existing| {
        let mut updates = existing
            .cloned()
            .filter(Value::is_object)
            .unwrap_or_else(|| json!({}));
        updates[ANNOUNCEMENT_STATE_KEY] = announcement.clone();
        Ok(updates)
    })
}

fn empty_announcement(now: u64) -> Value {
    json!({
        "lastAttemptAt": now,
        "checkedAt": null,
        "updateAvailable": false,
        "version": null,
        "releaseUrl": null,
        "changelog": []
    })
}

fn module_is_explicitly_disabled(config: &Value) -> bool {
    config
        .pointer("/modules/updates/enabled")
        .and_then(Value::as_bool)
        == Some(false)
}

fn eligible_for_release_news(config: &Value) -> bool {
    module_is_explicitly_disabled(config)
}

fn public_announcement(value: Value) -> Value {
    if value["updateAvailable"].as_bool() == Some(true) && value["version"].as_str().is_some() {
        json!({
            "updateAvailable": true,
            "latestVersion": value["version"],
            "releaseUrl": value["releaseUrl"],
            "changelog": value["changelog"].as_array().cloned().unwrap_or_default()
        })
    } else {
        json!({"updateAvailable": false})
    }
}

fn release_changelog(body: &str) -> Vec<String> {
    let lines = body.lines().map(str::trim).collect::<Vec<_>>();
    let has_changelog_heading = lines.iter().any(|line| {
        line.trim_matches('#')
            .trim()
            .eq_ignore_ascii_case("changelog:")
    });
    let mut in_changelog = !has_changelog_heading;
    lines
        .into_iter()
        .filter_map(|line| {
            if line
                .trim_matches('#')
                .trim()
                .eq_ignore_ascii_case("changelog:")
            {
                in_changelog = true;
                return None;
            }
            if line
                .trim_start_matches('*')
                .trim()
                .to_ascii_lowercase()
                .starts_with("full changelog")
            {
                in_changelog = false;
                return None;
            }
            if !in_changelog || line.is_empty() {
                return None;
            }
            let entry = line
                .strip_prefix("* ")
                .or_else(|| line.strip_prefix("- "))
                .or_else(|| line.strip_prefix("+ "))
                .unwrap_or(line)
                .trim();
            (!entry.is_empty()).then(|| entry.chars().take(2_000).collect::<String>())
        })
        .take(50)
        .collect()
}

fn installed_release(root: &Path) -> (String, u64) {
    let value = fs::read(shroudforge_loader_config::paths::version_file(root))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok());
    let version = value
        .as_ref()
        .and_then(|value| value["version"].as_str())
        .unwrap_or(env!("CARGO_PKG_VERSION"))
        .to_owned();
    let build = value
        .as_ref()
        .and_then(|value| {
            value["build"]
                .as_u64()
                .or_else(|| value["build"].as_str()?.parse().ok())
        })
        .unwrap_or(0);
    (version, build)
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
