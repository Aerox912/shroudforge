use super::*;

pub struct ResourceInfo {
    pub resource_id: ResourceId,
    pub(super) source: ResourceSource,
    pub(super) creation_session: u64,
    pub(super) runtime_snapshot: OnceCell<Rc<[u8]>>,
    pub(super) runtime_write_fault: RefCell<Option<String>>,

    pub(super) original_value: OnceCell<Option<MappedValue>>,
    pub(super) value: RefCell<Option<LuaValue>>,
    pub(super) is_dirty: Cell<bool>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ResourceSource {
    KfcFile,
    AssetTransaction,
    LoadedRuntime,
}

impl ResourceInfo {
    pub fn source_name(&self) -> &'static str {
        match self.source {
            ResourceSource::KfcFile => "kfc-file",
            ResourceSource::AssetTransaction => "asset-transaction",
            ResourceSource::LoadedRuntime => "loaded-runtime",
        }
    }

    pub fn get_lua_value(&self, lua: &mlua::Lua) -> mlua::Result<LuaValue> {
        if let Some(value) = self.value.borrow().as_ref() {
            return Ok(value.clone());
        }

        let value = match self.get_mapped_value(lua)? {
            Some(value) => convert_value_to_lua(value, lua)?,
            None => LuaValue::Nil,
        };

        self.value.replace(Some(value.clone()));

        Ok(value)
    }

    pub fn set_lua_value(&self, lua_value: LuaValue, lua: &mlua::Lua) -> mlua::Result<()> {
        if self.source == ResourceSource::LoadedRuntime {
            return Err(LuaError::generic("loaded runtime resources are read-only"));
        }
        let app_state = lua.app_data_ref::<AppState>().unwrap();
        if !app_state.has_feature(AppFeatures::PATCH) {
            return Err(LuaError::generic("game.assets.write is unavailable"));
        }
        let type_registry = app_state.type_registry();

        let r#type = kfc_format::type_for_resource(type_registry, &self.resource_id)
            .ok_or_else(|| LuaError::generic("resource type not found in current game registry"))?;

        let lua_value = validate_and_clone_lua_value(
            &lua_value,
            &TypeHandle::new(type_registry.clone(), r#type.index),
            lua,
        )
        .map_err(LuaError::external)?;

        self.value.replace(Some(lua_value));
        self.is_dirty.replace(true);

        Ok(())
    }

    pub fn apply(&self, lua: &mlua::Lua) -> mlua::Result<Option<Value>> {
        if self.source == ResourceSource::LoadedRuntime {
            return Ok(None);
        }
        let app_state = lua.app_data_ref::<AppState>().unwrap();
        let type_registry = app_state.type_registry();

        let lua_value = self.value.borrow();
        let lua_value = match lua_value.as_ref() {
            Some(value) => value,
            None => return Ok(None),
        };

        if !self.is_dirty.get() && !is_dirty_lua_value(lua_value)? {
            return Ok(None);
        }

        let r#type = kfc_format::type_for_resource(type_registry, &self.resource_id)
            .ok_or_else(|| LuaError::generic("resource type not found in current game registry"))?;

        let value = convert_lua_to_value(
            lua_value,
            &TypeHandle::new(type_registry.clone(), r#type.index),
        )
        .map_err(LuaError::external)?;

        Ok(Some(value))
    }

    fn get_mapped_value(&self, lua: &mlua::Lua) -> mlua::Result<Option<&MappedValue>> {
        let app_state = lua.app_data_ref::<AppState>().unwrap();

        self.original_value
            .get_or_try_init(|| {
                let bytes = match self.source {
                    ResourceSource::KfcFile => app_state
                        .reader()
                        .read_resource(&self.resource_id)?
                        .map(|bytes| Rc::<[u8]>::from(bytes)),
                    ResourceSource::AssetTransaction => None,
                    ResourceSource::LoadedRuntime => Some(
                        self.loaded_snapshot_bytes()
                            .map_err(LuaError::generic)?
                            .clone(),
                    ),
                };
                match bytes {
                    Some(value) => {
                        let type_registry = app_state.type_registry();
                        let r#type =
                            kfc_format::type_for_resource(type_registry, &self.resource_id)
                                .ok_or_else(|| {
                                    LuaError::generic(
                                        "resource type not found in current game registry",
                                    )
                                })?;

                        let value = MappedValue::from_bytes(type_registry, r#type, &value)
                            .map_err(LuaError::external)?;

                        Ok(Some(value))
                    }
                    None => Ok(None),
                }
            })
            .map(|v| v.as_ref())
    }
}

impl AppState {
    #[inline]
    pub fn kfc_file(&self) -> &KFCFile {
        &self.ref_file
    }

    #[inline]
    pub fn reader(&self) -> RefMut<'_, KFCCursor<KFCReader>> {
        self.reader.borrow_mut()
    }

    #[inline]
    pub fn take_writer(&self) -> mlua::Result<KFCWriter<Rc<KFCFile>, Rc<TypeRegistry>>> {
        self.writer
            .borrow_mut()
            .take()
            .ok_or_else(|| LuaError::generic("typed asset writer is unavailable in this phase"))
    }

    pub fn commit_assets(&self) -> mlua::Result<()> {
        if !self.asset_transaction.get() {
            return Err(LuaError::generic("typed asset transaction is unavailable"));
        }
        if self.prepared_asset_output {
            // Prepared output remains private in its separate stage and cannot
            // be published through the normal session commit entrypoint.
            self.asset_transaction.set(false);
            return Ok(());
        }
        if self.defer_current_commit {
            // Validated output is now owned by the outer pending receipt.
            self.asset_transaction.set(false);
            return Ok(());
        }
        let commit = if self.generated_asset_transaction {
            shroudforge_game_assets::transaction::commit_generated
        } else {
            shroudforge_game_assets::transaction::commit
        };
        commit(self.env.game_dir().as_std_path(), &self.asset_stem).map_err(LuaError::external)?;
        self.asset_transaction.set(false);
        Ok(())
    }

    pub fn get_cached_resources(&self) -> Vec<Rc<ResourceInfo>> {
        self.resources.borrow().values().cloned().collect()
    }

    pub fn asset_resource_ids(&self) -> Vec<ResourceId> {
        let file = self.kfc_file();
        let transaction_ids = self.transaction_resource_ids();
        let mut seen = HashSet::with_capacity(file.resources().len() + transaction_ids.len());
        let mut ids = Vec::with_capacity(file.resources().len() + transaction_ids.len());

        for id in file.resources().keys() {
            if seen.insert(*id) {
                ids.push(*id);
            }
        }

        for id in transaction_ids {
            if seen.insert(id) {
                ids.push(id);
            }
        }

        ids
    }

    pub fn asset_resource_ids_by_type(&self, qualified_name: &str) -> Vec<ResourceId> {
        let Some(metadata) = self
            .type_registry
            .get_by_name(kfc::reflection::LookupKey::Qualified(qualified_name))
        else {
            return Vec::new();
        };

        let file = self.kfc_file();
        let transaction_ids = self.transaction_resource_ids();
        let mut ids = kfc_format::resources_by_type(file, &self.type_registry, qualified_name);
        let mut seen = ids.iter().copied().collect::<HashSet<_>>();

        for id in transaction_ids {
            if id.type_hash() == metadata.qualified_hash && seen.insert(id) {
                ids.push(id);
            }
        }

        ids
    }

    pub fn asset_resource_type_indices(&self) -> Vec<TypeIndex> {
        let mut indices = kfc_format::resource_type_indices(self.kfc_file(), &self.type_registry);

        for id in self.transaction_resource_ids() {
            if let Some(metadata) = kfc_format::type_for_resource(&self.type_registry, &id) {
                if !indices.contains(&metadata.index) {
                    indices.push(metadata.index);
                }
            }
        }

        indices
    }

    fn transaction_resource_ids(&self) -> Vec<ResourceId> {
        let mut ids = self
            .resources
            .borrow()
            .values()
            .filter(|info| info.source == ResourceSource::AssetTransaction)
            .map(|info| info.resource_id)
            .collect::<Vec<_>>();
        ids.sort_unstable();
        ids
    }

    pub fn get_resource_info(&self, guid: &ResourceId) -> Option<Rc<ResourceInfo>> {
        match self.resources.borrow_mut().entry(*guid) {
            Entry::Occupied(entry) => Some(entry.get().clone()),
            Entry::Vacant(entry) => {
                if !self.ref_file.resources().contains_key(guid) {
                    return None;
                }

                let info = ResourceInfo {
                    resource_id: *guid,
                    source: ResourceSource::KfcFile,
                    creation_session: 0,
                    runtime_snapshot: OnceCell::new(),
                    runtime_write_fault: RefCell::new(None),
                    original_value: OnceCell::new(),
                    value: RefCell::default(),
                    is_dirty: Cell::new(false),
                };
                let info = Rc::new(info);

                entry.insert(info.clone());

                Some(info)
            }
        }
    }

    pub fn add_resource(
        &self,
        value: &LuaValue,
        guid: &ResourceId,
        lua: &mlua::Lua,
    ) -> mlua::Result<()> {
        if !self.has_feature(AppFeatures::PATCH) {
            return Err(LuaError::generic("game.assets.write is unavailable"));
        }
        let mut resources = self.resources.borrow_mut();

        if resources.contains_key(guid) {
            return Err(LuaError::generic(format!(
                "resource with GUID {guid} already exists"
            )));
        }

        let r#type = TypeHandle::new(
            self.type_registry.clone(),
            kfc_format::type_for_resource(&self.type_registry, guid)
                .ok_or_else(|| {
                    LuaError::generic("resource type not found in current game registry")
                })?
                .index,
        );
        let value =
            validate_and_clone_lua_value(value, &r#type, lua).map_err(LuaError::external)?;

        let info = Rc::new(ResourceInfo {
            resource_id: *guid,
            source: ResourceSource::AssetTransaction,
            creation_session: 0,
            runtime_snapshot: OnceCell::new(),
            runtime_write_fault: RefCell::new(None),
            original_value: OnceCell::new(),
            value: RefCell::new(Some(value)),
            is_dirty: Cell::new(true),
        });

        resources.insert(*guid, info);

        Ok(())
    }

    pub fn create_resource(
        &self,
        value: &LuaValue,
        type_index: TypeIndex,
        lua: &mlua::Lua,
    ) -> mlua::Result<ResourceId> {
        if !self.has_feature(AppFeatures::PATCH) {
            return Err(LuaError::generic("game.assets.write is unavailable"));
        }
        let resources = self.resources.borrow_mut();
        let mut guid: ResourceId;

        loop {
            let uuid = uuid::Uuid::new_v4().into_bytes();

            guid =
                kfc_format::resource_id_for_type(&self.type_registry, type_index, uuid.into(), 0)
                    .ok_or_else(|| {
                    LuaError::generic("resource type not found in current game registry")
                })?;

            if !resources.contains_key(&guid) {
                break;
            }
        }

        drop(resources);

        self.add_resource(value, &guid, lua)?;

        Ok(guid)
    }

    pub fn get_content(&self, guid: &ContentHash) -> mlua::Result<Option<Vec<u8>>> {
        if let Some(value) = self.new_contents.borrow().get(guid) {
            // OPTIMIZE: cloning here can be quite expensive
            Ok(Some(value.clone()))
        } else {
            // OPTIMIZE: consider using a reader for this
            Ok(self.reader.borrow_mut().read_content(guid)?)
        }
    }

    pub fn has_content(&self, guid: &ContentHash) -> bool {
        self.new_contents.borrow().contains_key(guid) || self.ref_file.contents().contains_key(guid)
    }

    pub fn create_content(&self, data: &[u8]) -> mlua::Result<ContentHash> {
        if !self.has_feature(AppFeatures::PATCH) {
            return Err(LuaError::generic("game.assets.write is unavailable"));
        }
        let guid = ContentHash::from_data(data);

        if let hash_map::Entry::Vacant(e) = self.new_contents.borrow_mut().entry(guid) {
            self.writer
                .borrow_mut()
                .as_mut()
                .expect("KFCWriter no longer available")
                .write_content(&guid, data)?;

            // TODO: attach a reader to the writer to avoid cloning
            e.insert(data.to_vec());
        }

        Ok(guid)
    }
}

#[cfg(test)]
mod transaction_visibility_tests {
    use super::*;
    use kfc::{
        container::{KFCFile, KFCWriter},
        guid::{ContentHash, Guid, ResourceId},
        reflection::TypeRegistry,
    };
    use sha2::Digest;
    use std::{fs, path::PathBuf};

    struct TempRoot(PathBuf);

    impl TempRoot {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "shroudforge-asset-transaction-visibility-{}",
                uuid::Uuid::new_v4()
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    struct Fixture {
        _root: TempRoot,
        environment: mod_loader::ModEnvironment,
        registry: TypeRegistry,
    }

    impl Fixture {
        fn new(mods: &[(&str, &str)]) -> Self {
            let temp = TempRoot::new();
            let root = &temp.0;
            let mods_root = root.join("mods");
            fs::create_dir_all(&mods_root).unwrap();

            for (id, source) in mods {
                let package = mods_root.join(id);
                fs::create_dir_all(package.join("src")).unwrap();
                fs::write(
                    package.join("mod.json"),
                    serde_json::to_vec(&serde_json::json!({
                        "id": id,
                        "name": id,
                        "version": "1.0.0",
                        "dependencies": [],
                        "capabilities": ["patch"]
                    }))
                    .unwrap(),
                )
                .unwrap();
                fs::write(package.join("src/mod.lua"), source).unwrap();
            }

            let root = fs::canonicalize(root).unwrap();
            let environment = mod_loader::ModEnvironment::load(root.to_str().unwrap()).unwrap();
            let mut registry = crate::env::registry::tests::fixture();
            registry.version = "asset-transaction-fixture".into();

            let executable = b"synthetic executable identity";
            fs::write(root.join("enshrouded.exe"), executable).unwrap();
            fs::write(root.join("enshrouded.kfc"), []).unwrap();
            fs::write(root.join("enshrouded.kfc_resources"), []).unwrap();

            let original_content = b"original localization";
            let content_hash = ContentHash::from_data(original_content);
            let u32_type = registry
                .iter()
                .find(|metadata| metadata.qualified_name == "test::U32")
                .unwrap();
            let original_resource = ResourceId::new(
                Guid::parse("00000000-0000-0000-0000-000000000001").unwrap(),
                u32_type.qualified_hash,
                0,
            );
            let mut writer =
                KFCWriter::<KFCFile, _>::new(&root, "enshrouded", &registry, &registry.version)
                    .unwrap();
            writer
                .write_resource(&original_resource, &7_u32.to_le_bytes())
                .unwrap();
            writer
                .write_content(&content_hash, original_content)
                .unwrap();
            writer.finalize().unwrap();

            fs::create_dir_all(environment.cache_dir().as_std_path()).unwrap();
            fs::write(
                environment.cache_dir().join("types-enshrouded.json"),
                serde_json::to_vec(&registry).unwrap(),
            )
            .unwrap();
            let identity = serde_json::json!({
                "schemaVersion": 2,
                "executableSha256": format!("{:x}", sha2::Sha256::digest(executable)),
                "reflectionExtractor": "kfc-a404b15cde40fc7bd43d812bb460ce40d2a9d3b1"
            });
            fs::write(
                environment
                    .cache_dir()
                    .join("types-enshrouded.identity.json"),
                serde_json::to_vec(&identity).unwrap(),
            )
            .unwrap();

            Self {
                _root: temp,
                environment,
                registry,
            }
        }

        fn app_state(&self, patch: bool) -> AppState {
            let game_file = KFCFile::from_path(self._root.0.join("enshrouded.kfc"), false).unwrap();
            let schema = kfc::schema_from_registry(&self.registry, &game_file);
            let api = crate::ShroudForgeApi::new(
                shroudforge_compatibility::Compatibility::default().resolve(schema),
            );
            AppState::new(
                self.environment.clone(),
                Some(api),
                crate::RunArgs {
                    file_name: "enshrouded".into(),
                    options: crate::RunOptions {
                        skip_cache: true,
                        force_patch: true,
                        is_server: Some(false),
                        patch,
                        phase: crate::RuntimePhase::Pregame,
                        ..Default::default()
                    },
                },
                &crate::cache::CacheDiff::new_dirty(),
            )
            .unwrap()
        }

        fn run_pair(&self) -> mlua::Result<()> {
            let producer = self.environment.mod_registry().get("a-producer").unwrap();
            let consumer = self.environment.mod_registry().get("b-consumer").unwrap();
            let runner = crate::runner::LuaModRunner::new(self.app_state(true))?;
            runner.setup([producer, consumer])?;
            runner.run()
        }

        fn run_pair_with_unrelated_loaded_resource(&self) -> mlua::Result<()> {
            let producer = self.environment.mod_registry().get("a-producer").unwrap();
            let consumer = self.environment.mod_registry().get("b-consumer").unwrap();
            let runner = crate::runner::LuaModRunner::new(self.app_state(true))?;
            runner.setup([producer, consumer])?;

            let u32_type = self
                .registry
                .iter()
                .find(|metadata| metadata.qualified_name == "test::U32")
                .unwrap();
            let loaded_resource = ResourceId::new(
                Guid::parse("22222222-2222-2222-2222-222222222222").unwrap(),
                u32_type.qualified_hash,
                0,
            );
            let app_state = runner.lua.app_data_ref::<AppState>().unwrap();
            app_state.resources.borrow_mut().insert(
                loaded_resource,
                Rc::new(ResourceInfo {
                    resource_id: loaded_resource,
                    source: ResourceSource::LoadedRuntime,
                    creation_session: 0,
                    runtime_snapshot: OnceCell::new(),
                    runtime_write_fault: RefCell::new(None),
                    original_value: OnceCell::new(),
                    value: RefCell::default(),
                    is_dirty: Cell::new(false),
                }),
            );
            drop(app_state);

            runner.run()
        }
    }

    #[test]
    fn created_content_is_visible_to_a_later_mod_without_changing_absence_behavior()
    -> mlua::Result<()> {
        let payload = "created localization";
        let content_hash = ContentHash::from_data(payload.as_bytes()).to_string();
        let absent_hash = ContentHash::from_data(b"not present").to_string();
        let producer = format!(
            r#"
            local payload = "{payload}"
            local created = game.assets.create_content(buffer.wrap(payload))
            assert(created.guid == "{content_hash}", "created content hash")
            return {{}}
            "#
        );
        let consumer = format!(
            r#"
            local created = game.assets.get_content("{content_hash}")
            assert(created ~= nil, "created content should be visible to the next mod")
            assert(created.guid == "{content_hash}")
            assert(created.size == {size})
            assert(created:read_data():tail() == {size})
            assert(game.assets.get_content("{absent_hash}") == nil,
                "missing content remains nil")
            return {{}}
            "#,
            size = payload.len(),
        );
        let fixture = Fixture::new(&[("a-producer", &producer), ("b-consumer", &consumer)]);

        fixture.run_pair()
    }

    #[test]
    fn created_resources_join_file_enumerations_after_original_resources_once() -> mlua::Result<()>
    {
        let created_guid = "11111111-1111-1111-1111-111111111111";
        let producer = format!(
            r#"
            game.assets.create_resource(13, "test::U32")
            game.assets.create_resource({{ x = 17 }}, "test::Base", "{created_guid}", 0)
            return {{}}
            "#
        );
        let consumer = format!(
            r#"
            local originals_and_new = game.assets.get_resources_by_type("test::U32")
            assert(#originals_and_new == 2, "existing and transaction resources are both visible")
            assert(originals_and_new[1].source == "kfc-file", "source file order is retained")
            assert(originals_and_new[2].source == "asset-transaction", "new resource follows originals")
            assert(originals_and_new[1].guid ~= originals_and_new[2].guid)

            local created = game.assets.get_resources_by_type("test::Base")
            assert(#created == 1, "created resource is visible by its type")
            assert(created[1].guid == "{created_guid}")
            assert(created[1].source == "asset-transaction")
            assert(game.assets.get_resource("{created_guid}", "test::Base").guid == "{created_guid}")
            assert(#game.assets.get_resource_parts("{created_guid}", "test::Base") == 1)

            local all = game.assets.get_all_resources()
            assert(#all == 3, "all-resource enumeration includes both new resources")
            local source_count, transaction_count = 0, 0
            for _, resource in ipairs(all) do
                if resource.source == "kfc-file" then source_count = source_count + 1 end
                if resource.source == "asset-transaction" then transaction_count = transaction_count + 1 end
            end
            assert(source_count == 1, "the original resource appears once")
            assert(transaction_count == 2, "only transaction resources are merged")

            local types = game.assets.get_resource_types()
            assert(#types == 2, "a created-only type is enumerated")
            assert(types[1].qualified_name == "test::U32", "existing type order is preserved")
            assert(types[2].qualified_name == "test::Base", "created type is appended")
            return {{}}
            "#
        );
        let fixture = Fixture::new(&[("a-producer", &producer), ("b-consumer", &consumer)]);

        fixture.run_pair_with_unrelated_loaded_resource()
    }

    #[test]
    fn asset_writes_remain_denied_without_patch_capability() -> mlua::Result<()> {
        let fixture = Fixture::new(&[("writer", "return {}")]);
        let writer = fixture.environment.mod_registry().get("writer").unwrap();
        let runner = crate::runner::LuaModRunner::new(fixture.app_state(false))?;
        runner.setup([writer])?;
        let environment = runner.diagnostic_environment("writer")?;
        runner
            .lua
            .load(
                r#"
                local ok, err = pcall(function()
                    game.assets.create_content(buffer.wrap("forbidden"))
                end)
                assert(not ok and string.find(tostring(err), "game.assets.write is unavailable", 1, true))
                local resource_ok, resource_err = pcall(function()
                    game.assets.create_resource(1, "test::U32")
                end)
                assert(not resource_ok and string.find(tostring(resource_err), "game.assets.write is unavailable", 1, true))
                "#,
            )
            .set_name("asset-write-capability-denied")
            .set_environment(environment)
            .exec()?;
        Ok(())
    }
}
