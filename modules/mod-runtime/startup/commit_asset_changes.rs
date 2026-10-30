use crate::{
    logging::{debug, info},
    session::{AppFeatures, AppState},
};
pub(super) fn commit_asset_changes(lua: &mlua::Lua, app_state: &AppState) -> anyhow::Result<()> {
    if !app_state.has_feature(AppFeatures::PATCH) {
        return Ok(());
    }
    debug!("Committing typed asset changes...");

    let mut buf = Vec::new();
    let mut writer = app_state.take_writer()?;
    let resources = app_state.get_cached_resources();
    let mut applied_resources = 0;
    let mut expected_resources = Vec::new();

    for resource in resources.iter() {
        let value = resource.apply(lua)?;

        if let Some(value) = value {
            let r#type = kfc::kfc_format::type_for_resource(
                app_state.type_registry(),
                &resource.resource_id,
            )
            .expect("resource type must exist in current game registry");

            buf.clear();
            value
                .write_into(app_state.type_registry(), r#type, &mut buf)
                .unwrap();
            writer.write_resource(&resource.resource_id, &buf)?;
            expected_resources.push((resource.resource_id, buf.clone()));
            applied_resources += 1;
        }
    }

    writer.finalize()?;
    app_state.validate_generated_assets(&expected_resources)?;
    app_state.commit_assets()?;

    info!("Committed {} changed resources", applied_resources);
    Ok(())
}
