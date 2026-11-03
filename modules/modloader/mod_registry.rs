use crate::{FileSystem, ModManifest};
use parking_lot::{Mutex, MutexGuard};
use std::{collections::HashMap, ops::Deref, sync::Arc};

#[derive(Debug)]
pub(crate) struct ModInner {
    pub(crate) info: ModManifest,
    pub(crate) fs: Mutex<FileSystem>,
    pub(crate) package_binding: crate::package_lookup::ModPackageBinding,
}

#[derive(Debug, Clone)]
pub struct Mod {
    pub(crate) inner: Arc<ModInner>,
}

impl Mod {
    pub fn info(&self) -> &ModManifest {
        &self.inner.info
    }

    pub fn fs(&self) -> MutexGuard<'_, FileSystem> {
        self.inner.fs.lock()
    }

    /// Stable package identity captured when this installed directory was admitted.
    pub fn package_binding(&self) -> &crate::package_lookup::ModPackageBinding {
        &self.inner.package_binding
    }
}

#[derive(Debug, Default)]
pub struct ModRegistry {
    pub(crate) mods: HashMap<String, Mod>,
}

impl Deref for ModRegistry {
    type Target = HashMap<String, Mod>;

    fn deref(&self) -> &Self::Target {
        &self.mods
    }
}
