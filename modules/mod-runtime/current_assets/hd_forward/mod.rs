//! Read-only admission for the HD-only forward generation. No writer is linked
//! through this module. Shared contract/admission sources are copied verbatim
//! from the frozen manager-hdmap-forward producer and checked at handoff.
mod admission;
mod contract;
mod files;
mod texture;
use anyhow::{Result, ensure};
use contract::{Marker, Receipt, Request};
use std::{fs, path::Path};
const SOURCE_CLOSURE: &str = env!("SF_HD_FORWARD_SOURCE_CLOSURE");

pub(super) fn reject_hold(root: &Path) -> Result<()> {
    admission::no_hold(root)
}
pub(super) fn verify(root: &Path) -> Result<()> {
    let root = files::absolute(root)?;
    // Hold is checked before leases as well as after them. No malformed or
    // unknown owner can fall through to schema2/ordinary generation.
    admission::no_hold(&root)?;
    admission::defaults(&root)?;
    let _outer = super::manager_lease(&root)?;
    let _startup = files::lease(&root)?;
    admission::no_hold(&root)?;
    ensure!(
        !files::checked(&root, ".enshrouded.shroudforge-stage")?.try_exists()?,
        "native publication remains pending"
    );
    let raw = files::text(&files::checked(&root, contract::POLICY)?)?;
    let marker: Marker = serde_json::from_str(&raw)?;
    let request: Request = serde_json::from_str(&marker.request_json)?;
    let receipt: Receipt = serde_json::from_str(&marker.receipt_json)?;
    admission::marker(&raw, &request, &receipt, SOURCE_CLOSURE)?;
    ensure!(
        contract::same_root(&root, &request.root),
        "forward runtime root differs"
    );
    let admitted = contract::validate(&request)?;
    admission::inputs(&request, &admitted, admission::Package::After)?;
    admission::assets(
        &root,
        &admitted,
        &contract::effective_assets(&admitted, &receipt),
    )?;
    super::zoom_demand::publish(
        &root,
        admitted.origin.extended_zoom,
        &admitted.request.inputs["enshrouded.exe"].sha256,
    );
    let mask = u32::from(admitted.origin.preserved_loaders.contains_key("winmm.dll"))
        | (u32::from(
            admitted
                .origin
                .preserved_loaders
                .contains_key("dbgcore.dll"),
        ) << 1);
    super::LOADERS.with(|slot| *slot.borrow_mut() = Some((root, mask)));
    Ok(())
}
