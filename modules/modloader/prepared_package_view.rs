//! An explicit, immutable prepared-only namespace. Construction does not open
//! files, discover profiles, materialize overrides or qualify source provenance.
//! The prepared admission caller retains and finally verifies the source graph.
use crate::alias::{Path, PathBuf};
use parking_lot::Mutex;
use std::{
    collections::HashMap,
    fs::File,
    io::{self, Error, ErrorKind, Read},
    sync::Arc,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreparedReadCounts {
    pub charged_bytes: u64,
    pub returned_bytes: u64,
    pub calls: u64,
    pub finalizing: bool,
    pub terminal_io: bool,
}

#[derive(Debug)]
struct BudgetState {
    counts: PreparedReadCounts,
    max_bytes: u64,
    max_calls: u64,
    reserve_bytes: u64,
    reserve_calls: u64,
}

/// Shared by the entire admitted graph, never reset per package or phase.
#[derive(Debug, Clone)]
pub struct PreparedReadBudget(Arc<Mutex<BudgetState>>);

impl PreparedReadBudget {
    pub fn new(
        max_bytes: u64,
        max_calls: u64,
        reserve_bytes: u64,
        reserve_calls: u64,
    ) -> io::Result<Self> {
        if max_bytes < reserve_bytes || max_calls < reserve_calls {
            return Err(invalid("invalid final reserve"));
        }
        Ok(Self(Arc::new(Mutex::new(BudgetState {
            counts: PreparedReadCounts {
                charged_bytes: 0,
                returned_bytes: 0,
                calls: 0,
                finalizing: false,
                terminal_io: false,
            },
            max_bytes,
            max_calls,
            reserve_bytes,
            reserve_calls,
        }))))
    }
    pub fn counts(&self) -> PreparedReadCounts {
        self.0.lock().counts
    }
    pub fn begin_finalization(&self) -> io::Result<()> {
        let mut state = self.0.lock();
        if state.counts.finalizing || state.counts.terminal_io {
            return Err(invalid("finalization unavailable"));
        }
        state.counts.finalizing = true;
        Ok(())
    }
    fn debit(&self, bytes: u64, finalizing: bool) -> io::Result<()> {
        let mut state = self.0.lock();
        if state.counts.terminal_io || state.counts.finalizing != finalizing {
            return Err(invalid("read phase closed"));
        }
        let max_bytes = state.max_bytes - if finalizing { 0 } else { state.reserve_bytes };
        let max_calls = state.max_calls - if finalizing { 0 } else { state.reserve_calls };
        if bytes > max_bytes - state.counts.charged_bytes || state.counts.calls >= max_calls {
            return Err(invalid("prepared read budget"));
        }
        state.counts.charged_bytes += bytes;
        state.counts.calls += 1;
        Ok(())
    }
    fn terminal(&self) {
        self.0.lock().counts.terminal_io = true;
    }
    fn returned(&self, bytes: usize) {
        self.0.lock().counts.returned_bytes += bytes as u64;
    }
}

#[derive(Debug, Clone)]
pub struct PreparedSource(Source);

#[derive(Debug, Clone)]
enum Source {
    Bytes(Arc<[u8]>),
    Held { file: Arc<File>, bytes: u64 },
}

impl PreparedSource {
    /// Only small, hash-admitted overrides should use owned bytes. Large assets
    /// can remain in their original held files without another bulk allocation.
    pub fn bytes(bytes: Arc<[u8]>) -> Self {
        Self(Source::Bytes(bytes))
    }
    /// A capability supplied by the qualified lease, not a path to reopen.
    pub fn held(file: Arc<File>, bytes: u64) -> io::Result<Self> {
        let metadata = file.metadata()?;
        if !metadata.is_file() || metadata.len() != bytes {
            return Err(invalid("held source size/kind"));
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes() & 0x400 != 0 {
                return Err(invalid("held source reparse"));
            }
        }
        Ok(Self(Source::Held { file, bytes }))
    }
    pub fn len(&self) -> u64 {
        match &self.0 {
            Source::Bytes(b) => b.len() as u64,
            Source::Held { bytes, .. } => *bytes,
        }
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn reader(&self, budget: PreparedReadBudget, finalizing: bool) -> PreparedSourceReader {
        PreparedSourceReader {
            source: self.clone(),
            budget,
            position: 0,
            finalizing,
        }
    }
}

pub struct PreparedSourceReader {
    source: PreparedSource,
    budget: PreparedReadBudget,
    position: u64,
    finalizing: bool,
}
impl Read for PreparedSourceReader {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        // Virtual EOF does not probe the underlying file or debit an IO call.
        if buffer.is_empty() || self.position == self.source.len() {
            return Ok(0);
        }
        let n = buffer
            .len()
            .min(65_536)
            .min((self.source.len() - self.position) as usize);
        self.budget.debit(n as u64, self.finalizing)?;
        let result = match &self.source.0 {
            Source::Bytes(bytes) => {
                buffer[..n]
                    .copy_from_slice(&bytes[self.position as usize..self.position as usize + n]);
                Ok(n)
            }
            Source::Held { file, .. } => {
                #[cfg(windows)]
                {
                    use std::os::windows::fs::FileExt;
                    file.seek_read(&mut buffer[..n], self.position)
                }
                #[cfg(unix)]
                {
                    use std::os::unix::fs::FileExt;
                    file.read_at(&mut buffer[..n], self.position)
                }
                #[cfg(not(any(windows, unix)))]
                {
                    let _ = file;
                    Err(Error::new(ErrorKind::Unsupported, "held positioned reads"))
                }
            }
        };
        if let Ok(got) = &result {
            self.budget.returned(*got);
        }
        match result {
            Ok(got) if got == n => {
                self.position += got as u64;
                Ok(got)
            }
            Ok(_) => {
                self.budget.terminal();
                Err(Error::new(
                    ErrorKind::UnexpectedEof,
                    "short admitted source read; no retry",
                ))
            }
            Err(error) => {
                self.budget.terminal();
                Err(error)
            }
        }
    }
}

fn invalid(message: &str) -> Error {
    Error::new(ErrorKind::InvalidInput, message)
}

fn relative(path: &Path, root_allowed: bool) -> io::Result<()> {
    let value = path.as_str();
    if value.is_empty() && root_allowed {
        return Ok(());
    }
    if value.is_empty() || !value.is_ascii() || value.contains(['\\', ':']) {
        return Err(invalid("noncanonical prepared path"));
    }
    for part in value.split('/') {
        let stem = part.split('.').next().unwrap_or("").to_ascii_uppercase();
        if part.is_empty()
            || part == "."
            || part == ".."
            || part.ends_with([' ', '.'])
            || part
                .chars()
                .any(|c| c.is_control() || "<>\"|?*".contains(c))
            || ["CON", "PRN", "AUX", "NUL"].contains(&stem.as_str())
            || (stem.len() == 4
                && (stem.starts_with("COM") || stem.starts_with("LPT"))
                && matches!(stem.as_bytes()[3], b'1'..=b'9'))
        {
            return Err(invalid("unsafe prepared component"));
        }
    }
    Ok(())
}

/// One captured namespace entry. None denotes an admitted directory. Entries
/// must be in captured traversal order with parents before children, including
/// the empty root first. Replacing a source never adds or reorders an entry.
#[derive(Debug, Clone)]
pub struct PreparedPackageEntry {
    pub path: PathBuf,
    pub source: Option<PreparedSource>,
}

#[derive(Debug)]
pub struct PreparedPackageView {
    root: PathBuf,
    entries: Vec<PreparedPackageEntry>,
    indices: HashMap<PathBuf, usize>,
    children: HashMap<PathBuf, Vec<PathBuf>>,
    budget: PreparedReadBudget,
}

impl PreparedPackageView {
    pub fn new(
        root: PathBuf,
        entries: Vec<PreparedPackageEntry>,
        budget: PreparedReadBudget,
    ) -> io::Result<Self> {
        if !root.is_absolute() || entries.is_empty() || entries.len() > 10_000 {
            return Err(invalid("prepared namespace bounds"));
        }
        let mut indices: HashMap<PathBuf, usize> = HashMap::new();
        let mut folded = std::collections::HashSet::new();
        let mut children: HashMap<PathBuf, Vec<PathBuf>> = HashMap::new();
        for (index, entry) in entries.iter().enumerate() {
            relative(&entry.path, index == 0)?;
            if !folded.insert(entry.path.as_str().to_ascii_lowercase()) {
                return Err(invalid("duplicate prepared namespace"));
            }
            if index == 0 {
                if !entry.path.as_str().is_empty() || entry.source.is_some() {
                    return Err(invalid("prepared root"));
                }
            } else {
                let parent = entry
                    .path
                    .parent()
                    .ok_or_else(|| invalid("prepared parent"))?;
                let previous = indices
                    .get(parent)
                    .ok_or_else(|| invalid("unadmitted parent"))?;
                if entries[*previous].source.is_some() {
                    return Err(invalid("prepared file descendant"));
                }
                children
                    .entry(parent.to_owned())
                    .or_default()
                    .push(entry.path.clone());
            }
            indices.insert(entry.path.clone(), index);
        }
        Ok(Self {
            root,
            entries,
            indices,
            children,
            budget,
        })
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    fn entry(&self, path: &Path) -> io::Result<Option<&PreparedPackageEntry>> {
        relative(path, true)?;
        Ok(self.indices.get(path).map(|i| &self.entries[*i]))
    }
    pub fn files(&self) -> Vec<io::Result<PathBuf>> {
        self.entries
            .iter()
            .map(|entry| Ok(entry.path.clone()))
            .collect()
    }
    pub fn read_directory(&self, path: &Path) -> io::Result<Vec<io::Result<PathBuf>>> {
        match self.entry(path)? {
            Some(entry) if entry.source.is_none() => Ok(self
                .children
                .get(path)
                .into_iter()
                .flatten()
                .cloned()
                .map(Ok)
                .collect()),
            Some(_) => Err(Error::from(ErrorKind::NotADirectory)),
            None => Err(Error::from(ErrorKind::NotFound)),
        }
    }
    pub fn read_file(&self, path: &Path) -> io::Result<PreparedSourceReader> {
        match self.entry(path)? {
            Some(entry) => entry
                .source
                .as_ref()
                .map(|source| source.reader(self.budget.clone(), false))
                .ok_or_else(|| Error::from(ErrorKind::IsADirectory)),
            None => Err(Error::from(ErrorKind::NotFound)),
        }
    }
    pub fn exists(&self, path: &Path) -> bool {
        self.entry(path).is_ok_and(|entry| entry.is_some())
    }
    pub fn is_file(&self, path: &Path) -> bool {
        self.entry(path)
            .is_ok_and(|entry| entry.is_some_and(|e| e.source.is_some()))
    }
    pub fn is_directory(&self, path: &Path) -> bool {
        self.entry(path)
            .is_ok_and(|entry| entry.is_some_and(|e| e.source.is_none()))
    }
}
