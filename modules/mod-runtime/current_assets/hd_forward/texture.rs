use anyhow::{Result, ensure};
use serde_json::{Value, json};

pub const PAYLOAD_SHA256: &str = "fdf0a3745a6be2287f81b8b0c50ef0ea4e5b3dbaaea9240563a3812c22b74748";
pub const PAYLOAD_BYTES: u64 = 89_478_512;
pub const GENERATION: &str = "1273043|^/game38/branches/release_1.0|2026-10-20T14:55:29.904125Z";
pub const TEXTURE_GUID: &str = "01bcbd07-bdbf-41c0-9999-5d58fb1a3aa1";
pub const TEXTURE_TYPE: u32 = 0xf9e86997;

pub fn content_hash() -> Value {
    json!({"size":89478512,"hash0":453950516,"hash1":3431109454u32,"hash2":231543156})
}

pub fn forward(before: &Value) -> Result<Value> {
    ensure!(
        before["debugName"] == "_map_shader_parameters_baseGradient"
            && before["type"] == "Texture2D"
            && before["format"] == "R8G8B8A8_unorm"
            && before["size"]["x"] == 2048
            && before["size"]["y"] == 2048
            && before["levelCount"] == 12
            && before["isTiled"] == false
            && before["data"]["hash"]
                == json!({"size":22369620,"hash0":1960727610,"hash1":4155555733u32,"hash2":1567208542}),
        "texture differs from the qualified current 2K contribution"
    );
    let mut after = before.clone();
    for (path, value) in [
        ("/data/hash", content_hash()),
        ("/format", json!("BC7_unorm_block")),
        ("/levelCount", json!(14)),
        ("/size/x", json!(8192)),
        ("/size/y", json!(8192)),
    ] {
        *after.pointer_mut(path).expect("validated existing field") = value;
    }
    Ok(after)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn current() -> Value {
        json!({"debugName":"_map_shader_parameters_baseGradient",
          "size":{"x":2048,"y":2048},"type":"Texture2D",
          "format":"R8G8B8A8_unorm","levelCount":12,"isTiled":false,
          "data":{"hash":{"size":22369620,"hash0":1960727610,"hash1":4155555733u32,"hash2":1567208542},"offset":123},
          "unrelated":{"sentinel":710,"flags":[true,false]}})
    }

    #[test]
    fn five_owned_fields_change_and_every_other_field_survives() {
        let before = current();
        let after = forward(&before).unwrap();
        assert_eq!(after["size"], json!({"x":8192,"y":8192}));
        assert_eq!(after["format"], "BC7_unorm_block");
        assert_eq!(after["levelCount"], 14);
        assert_eq!(
            after["data"]["hash"],
            json!({"size":89478512,"hash0":453950516,"hash1":3431109454u32,"hash2":231543156})
        );
        let mut masked = after;
        for path in ["/data/hash", "/size/x", "/size/y", "/format", "/levelCount"] {
            *masked.pointer_mut(path).unwrap() = before.pointer(path).unwrap().clone();
        }
        assert_eq!(masked, before);
    }

    #[test]
    fn changed_or_already_bc7_input_is_refused_without_mutation() {
        for (path, bad) in [
            ("/data/hash/hash0", json!(9)),
            ("/format", json!("BC7_srgb_block")),
            ("/size/x", json!(4096)),
            ("/levelCount", json!(14)),
            ("/isTiled", json!(true)),
            ("/type", json!("Texture1D")),
            ("/debugName", json!("another texture")),
        ] {
            let mut before = current();
            *before.pointer_mut(path).unwrap() = bad;
            let retained = before.clone();
            assert!(
                forward(&before).is_err(),
                "unqualified input accepted at {path}"
            );
            assert_eq!(before, retained);
        }
    }
}
