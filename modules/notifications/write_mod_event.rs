use shroudforge_logging::{LogLevel, write_activity};
use std::{
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

pub fn write_event_notice(
    root: &Path,
    id: &str,
    mod_id: &str,
    kind: &str,
    level: LogLevel,
    name: &str,
    version: &str,
    ecosystem: Option<&str>,
    previous_version: Option<&str>,
    changelog: Option<&serde_json::Value>,
) {
    let values = serde_json::json!({
        "name": name,
        "version": version,
        "previousVersion": previous_version
    });
    let payload = serde_json::json!({
        "id": id,
        "modId": mod_id,
        "ecosystem": ecosystem,
        "title": "",
        "message": "",
        "level": level.as_str(),
        "kind": kind,
        "values": values,
        "changelog": changelog.cloned().unwrap_or_else(|| serde_json::json!([])),
        "actions": [{"id": "open-mod", "kind": "open-mod", "target": mod_id}],
        "updatedAt": SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
    });
    if let Err(error) = crate::write_event(root, &format!("event-{id}"), &payload) {
        write_activity(
            root,
            "news",
            "Publish notification",
            "Failed",
            Some(&error),
            "error",
        );
    }
}
