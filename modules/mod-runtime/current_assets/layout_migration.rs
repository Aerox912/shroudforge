//! Exact HD Map 0.2.0 to 0.2.1 GPC transition. The surrounding Migration
//! remains the sole authority for predecessor state and output inventory.
use super::*;

const GPC: &str = "7df551f7-ddd1-52b0-8ba6-d09be35dbccc";
const OLD: &str = "bf43423e8f39870cce1cdbbdfcbef76337367f889c93bf782019155d1fb5a51b";
const NEW: &str = "72c086eecc5a1852c50367109c0126f86102a1e472b7c07546fa4885c078ebe8";
// Derived from the exact reviewed archive using ShroudForge's existing
// remove(value) package normalization, which intentionally differs from the
// owner's value=true pin and preserves the accepted recipe fingerprint rules.
const PACKAGE: &str = "92b515e9587ccd44911208786b2c643cbe4d1d890f34695a668204ae2dc78343";
const TYPE_HASH: u32 = 0x647628d6;
const OFFSET: usize = 548;

/// Empty marker only. State, selection, policy and inventory authority remain
/// in the existing exact Migration contract; owner-only fields are rejected.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Contract {}

pub(super) fn package(inputs: &BTreeMap<String, String>) -> Result<()> {
    ensure!(
        hash(&serde_json::to_vec(inputs)?) == PACKAGE,
        "layout migration requires the exact reviewed HD Map 0.2.1 package"
    );
    Ok(())
}

pub(super) fn gpc_id() -> ResourceId {
    ResourceId::parse(GPC, TYPE_HASH, 0).unwrap()
}

pub(super) fn gpc_applied_digest(policy: &Policy) -> Result<()> {
    let mut found = 0;
    for record in &policy.resources {
        if record.id == GPC {
            found += 1;
            ensure!(
                record.type_hash == TYPE_HASH && record.applied_sha256 == NEW,
                "layout successor policy does not pin the reviewed GPC postimage"
            );
        }
    }
    ensure!(
        found == 1,
        "layout successor must select one exact GPC resource"
    );
    Ok(())
}

pub(super) fn old_applied_digest() -> &'static str {
    OLD
}

pub(super) fn corrected(bytes: &[u8]) -> Result<Vec<u8>> {
    corrected_with_pins(bytes, OLD, NEW)
}

fn corrected_with_pins(bytes: &[u8], before_hash: &str, after_hash: &str) -> Result<Vec<u8>> {
    ensure!(
        hash(bytes) == before_hash && bytes.get(OFFSET) == Some(&1),
        "layout predecessor GPC bytes mismatch"
    );
    let mut next = bytes.to_vec();
    next[OFFSET] = 2;
    ensure!(
        hash(&next) == after_hash,
        "layout corrected GPC bytes mismatch"
    );
    Ok(next)
}

pub(super) fn correction(root: &Path) -> Result<Vec<u8>> {
    let bytes = KFCReader::new(root, STEM)?
        .into_cursor()?
        .read_resource(&gpc_id())?
        .context("layout predecessor GPC resource absent")?;
    corrected(&bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_package_tree_pin_matches_reviewed_normalized_subtree() -> Result<()> {
        let inputs = BTreeMap::from([
            (
                "mods/esm-real-map/CREDITS.md".into(),
                "5d2a7d2b75d3198fd965991812e69dfee590cb183b3c65fefeeafdcfe9e6c750".into(),
            ),
            (
                "mods/esm-real-map/README.md".into(),
                "44bb2c0c2e9dc102133345d93d8954b6fc75806d6ab192a3647d8e42d72227e2".into(),
            ),
            (
                "mods/esm-real-map/assets/map-shader.bin".into(),
                "60de8c34784d0452e06fe4d1c5b1f26d81e7cf9b3a8c8689e85cd79ff8ccddf4".into(),
            ),
            (
                "mods/esm-real-map/assets/map.rgba".into(),
                "90bc074aafe1a5a4bcd250ef9cb28623b12f1c280602a066335af90f423a5fb0".into(),
            ),
            (
                "mods/esm-real-map/extended.mod.json".into(),
                "d92ef0798f7f410fff6fa5fa1f138d96f40acb53fb683b12c016a8171e8beed0".into(),
            ),
            (
                "mods/esm-real-map/mod.json".into(),
                "17415e7ec776aa69a537b16f1bd4a95c5afb45087c72828b64dfc40279dfa2a7".into(),
            ),
            (
                "mods/esm-real-map/src/mod.lua".into(),
                "46330922af115df32adaecc6af73172ae3a7036c75de870bb29131195aea3c44".into(),
            ),
            (
                "mods/esm-real-map/src/profile.lua".into(),
                "09b319ef1481356bb5c04ddfe2c2d96e3734bf91b8a93f343c56ac817f229fe8".into(),
            ),
            (
                "mods/esm-real-map/src/zoom.lua".into(),
                "7e6095fe132eb909b028bb709e495439e8ac24b3cc16eda5b30a54788b65692c".into(),
            ),
        ]);
        package(&inputs)?;
        let mut wrong = inputs.clone();
        wrong.insert("mods/esm-real-map/extra.lua".into(), "0".repeat(64));
        assert!(package(&wrong).is_err());
        let mut missing = inputs;
        missing.remove("mods/esm-real-map/src/zoom.lua");
        assert!(package(&missing).is_err());
        Ok(())
    }

    #[test]
    fn pinned_gpc_correction_changes_only_the_reviewed_byte() -> Result<()> {
        let mut before = vec![0; 600];
        before[OFFSET] = 1;
        let mut expected = before.clone();
        expected[OFFSET] = 2;
        let actual = corrected_with_pins(&before, &hash(&before), &hash(&expected))?;
        assert_eq!(actual, expected);
        assert!(corrected_with_pins(&before, &"0".repeat(64), &hash(&expected)).is_err());
        let mut wrong_offset = before;
        wrong_offset[OFFSET - 1] = 2;
        assert!(
            corrected_with_pins(&wrong_offset, &hash(&wrong_offset), &hash(&expected)).is_err()
        );
        Ok(())
    }

    #[test]
    fn unreviewed_gpc_bytes_and_owner_contract_fields_are_refused() -> Result<()> {
        assert!(corrected(&vec![0; 600]).is_err());
        assert!(corrected(&vec![1; 600]).is_err());
        assert!(
            serde_json::from_value::<Contract>(serde_json::json!({"policy":"old.json"})).is_err()
        );
        assert!(
            serde_json::from_value::<Contract>(serde_json::json!({"stateSha256":"0".repeat(64)}))
                .is_err()
        );
        assert!(serde_json::from_value::<Contract>(serde_json::json!({})).is_ok());
        Ok(())
    }
}
