use std::path::Path;

pub fn reset_window_geometry(root: &Path, module: &str, component: &str) -> Result<(), String> {
    if !matches!(module, "modloaderUi" | "updates" | "debugConsole") {
        return Err(format!("unsupported window module: {module}"));
    }
    let keys = match component {
        "position" => ["x", "y"],
        "size" => ["width", "height"],
        _ => {
            return Err(format!(
                "unsupported window geometry component: {component}"
            ));
        }
    };

    shroudforge_loader_config::config::update_loader(root, |config| {
        let position = &mut config["modules"][module]["window"]["position"];
        let empty = if let Some(position) = position.as_object_mut() {
            for key in keys {
                position.remove(key);
            }
            position.is_empty()
        } else {
            false
        };
        if empty {
            *position = serde_json::Value::Null;
        }
        Ok(())
    })
}
