//! Ordered background cache service with validated envelopes and atomic replacement.

use bincode::Options;
use crossbeam_channel::{Receiver, Sender, TrySendError};
use log::{debug, info, warn};
use serde::ser::SerializeSeq;
use serde::{Deserialize, Serialize};
use squarebob_core::{DirEntry, LodExpandInfo};
use std::cell::Cell;
use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::thread::JoinHandle;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::path_key::{self, ScanRoot};

const CACHE_VERSION: u32 = 4;
const MAX_CACHE_BYTES: u64 = 1024 * 1024 * 1024;
const CACHE_COMMAND_CAPACITY: usize = 8;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct CacheQuality {
    pub complete: bool,
    pub errors: u64,
}

#[derive(Debug)]
pub struct CachedScan {
    pub version: u32,
    pub root_id: String,
    pub scan_path: String,
    pub timestamp: u64,
    pub quality: CacheQuality,
    pub tree: DirEntry,
}

#[derive(Debug, Serialize, Deserialize)]
struct FlatCachedScan {
    version: u32,
    root_id: String,
    scan_path: String,
    timestamp: u64,
    quality: CacheQuality,
    nodes: Vec<FlatNode>,
}

#[derive(Debug, Serialize, Deserialize)]
struct FlatNode {
    parent_index: Option<u32>,
    name: String,
    path: PathBuf,
    size: u64,
    own_size: u64,
    is_dir: bool,
    ext: String,
    file_count: u64,
    dir_count: u64,
    modified_time: Option<u64>,
    lod_expand: Option<LodExpandInfo>,
}

#[derive(Debug)]
pub struct PreparedCache {
    pub cached: CachedScan,
    pub ext_stats: Vec<(String, u64, u64)>,
    pub size_range: (u64, u64),
}

#[derive(Debug)]
pub enum CacheEvent {
    Loaded {
        generation: u64,
        root_id: String,
        result: Result<Option<PreparedCache>, String>,
    },
    Stored {
        generation: u64,
        root_id: String,
        result: Result<(), String>,
    },
    Deleted {
        generation: u64,
        root_id: String,
        result: Result<(), String>,
    },
}

enum CacheCommand {
    Load {
        generation: u64,
        root: ScanRoot,
    },
    Store {
        generation: u64,
        root: ScanRoot,
        bytes: Vec<u8>,
    },
    Delete {
        generation: u64,
        root: ScanRoot,
    },
    Shutdown,
}

/// Single ordered owner for all cache I/O.
///
/// Per-root generation watermarks prevent an old scan from recreating or
/// replacing a cache after a newer store/delete command.
pub struct CacheService {
    command_tx: Sender<CacheCommand>,
    event_rx: Receiver<CacheEvent>,
    worker: Option<JoinHandle<()>>,
}

impl CacheService {
    pub fn spawn() -> anyhow::Result<Self> {
        let (command_tx, command_rx) = crossbeam_channel::bounded(CACHE_COMMAND_CAPACITY);
        let (event_tx, event_rx) = crossbeam_channel::unbounded();
        let worker = std::thread::Builder::new()
            .name("scan-cache".into())
            .spawn(move || cache_worker(command_rx, event_tx))
            .map_err(|e| anyhow::anyhow!("failed to spawn cache service: {e}"))?;
        Ok(Self {
            command_tx,
            event_rx,
            worker: Some(worker),
        })
    }

    pub fn load(&self, generation: u64, root: ScanRoot) -> anyhow::Result<()> {
        self.send_command(CacheCommand::Load { generation, root })
    }

    pub fn store(&self, generation: u64, root: ScanRoot, bytes: Vec<u8>) -> anyhow::Result<()> {
        self.send_command(CacheCommand::Store {
            generation,
            root,
            bytes,
        })
    }

    pub fn delete(&self, generation: u64, root: ScanRoot) -> anyhow::Result<()> {
        self.send_command(CacheCommand::Delete { generation, root })
    }

    fn send_command(&self, command: CacheCommand) -> anyhow::Result<()> {
        match self.command_tx.try_send(command) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => {
                anyhow::bail!("cache command queue is full; retry after pending I/O completes")
            }
            Err(TrySendError::Disconnected(_)) => anyhow::bail!("cache service stopped"),
        }
    }

    pub fn try_iter(&self) -> crossbeam_channel::TryIter<'_, CacheEvent> {
        self.event_rx.try_iter()
    }
}

impl Drop for CacheService {
    fn drop(&mut self) {
        let _ = self.command_tx.send(CacheCommand::Shutdown);
        if let Some(worker) = self.worker.take()
            && let Err(payload) = worker.join()
        {
            warn!("cache service panicked during shutdown: {payload:?}");
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct CacheWatermark {
    generation: u64,
    deleted: bool,
}

impl CacheWatermark {
    fn accept_store(&mut self, generation: u64) -> bool {
        if generation < self.generation || (generation == self.generation && self.deleted) {
            return false;
        }
        *self = Self {
            generation,
            deleted: false,
        };
        true
    }

    fn accept_delete(&mut self, generation: u64) -> bool {
        if generation < self.generation {
            return false;
        }
        *self = Self {
            generation,
            deleted: true,
        };
        true
    }
}

fn cache_worker(command_rx: Receiver<CacheCommand>, event_tx: Sender<CacheEvent>) {
    let mut watermarks: HashMap<String, CacheWatermark> = HashMap::new();

    while let Ok(command) = command_rx.recv() {
        let event = match command {
            CacheCommand::Load { generation, root } => Some(CacheEvent::Loaded {
                generation,
                root_id: root.id().to_owned(),
                result: load_cache(&root).map_err(|e| format!("{e:#}")),
            }),
            CacheCommand::Store {
                generation,
                root,
                bytes,
            } => {
                let watermark = watermarks.entry(root.id().to_owned()).or_default();
                let result = if watermark.accept_store(generation) {
                    write_cache_bytes(&root, &bytes)
                } else {
                    Ok(())
                };
                Some(CacheEvent::Stored {
                    generation,
                    root_id: root.id().to_owned(),
                    result: result.map_err(|e| format!("{e:#}")),
                })
            }
            CacheCommand::Delete { generation, root } => {
                let watermark = watermarks.entry(root.id().to_owned()).or_default();
                let result = if watermark.accept_delete(generation) {
                    delete_cache(&root)
                } else {
                    Ok(())
                };
                Some(CacheEvent::Deleted {
                    generation,
                    root_id: root.id().to_owned(),
                    result: result.map_err(|e| format!("{e:#}")),
                })
            }
            CacheCommand::Shutdown => break,
        };

        if let Some(event) = event
            && event_tx.send(event).is_err()
        {
            break;
        }
    }
}

fn cache_dir() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "squarebob-rs")
        .map(|dirs| dirs.cache_dir().to_path_buf())
}

fn cache_path(root: &ScanRoot) -> Option<PathBuf> {
    cache_dir().map(|dir| dir.join(format!("{}.bin", root.id())))
}

fn legacy_cache_path(root: &ScanRoot) -> Option<PathBuf> {
    cache_dir().map(|dir| {
        dir.join(format!(
            "{}.bin",
            path_key::legacy_scan_path_id_hex(root.display())
        ))
    })
}

pub fn serialize_cache(
    root: &ScanRoot,
    tree: &DirEntry,
    quality: CacheQuality,
) -> anyhow::Result<Vec<u8>> {
    serialize_cache_ref(root, tree, quality)
}

/// Serialize a validated tree without cloning its owned strings or paths.
pub fn serialize_cache_ref(
    root: &ScanRoot,
    tree: &DirEntry,
    quality: CacheQuality,
) -> anyhow::Result<Vec<u8>> {
    let node_count = validate_tree(tree, root.path())?;
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let envelope = FlatCachedScanRef {
        version: CACHE_VERSION,
        root_id: root.id(),
        scan_path: root.display(),
        timestamp,
        quality,
        nodes: FlatTreeRef {
            root: tree,
            node_count,
        },
    };
    let mut writer = BoundedVecWriter::new(MAX_CACHE_BYTES)?;
    bincode::DefaultOptions::new()
        .with_fixint_encoding()
        .with_little_endian()
        .with_limit(MAX_CACHE_BYTES)
        .serialize_into(&mut writer, &envelope)?;
    Ok(writer.finish())
}

#[derive(Serialize)]
struct FlatCachedScanRef<'a> {
    version: u32,
    root_id: &'a str,
    scan_path: &'a str,
    timestamp: u64,
    quality: CacheQuality,
    nodes: FlatTreeRef<'a>,
}

struct FlatTreeRef<'a> {
    root: &'a DirEntry,
    node_count: usize,
}

#[derive(Serialize)]
struct FlatNodeRef<'a> {
    parent_index: Option<u32>,
    name: &'a str,
    path: &'a Path,
    size: u64,
    own_size: u64,
    is_dir: bool,
    ext: &'a str,
    file_count: u64,
    dir_count: u64,
    modified_time: Option<u64>,
    lod_expand: Option<&'a LodExpandInfo>,
}

impl Serialize for FlatTreeRef<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut sequence = serializer.serialize_seq(Some(self.node_count))?;
        let mut pending = Vec::new();
        pending.try_reserve(1).map_err(serde::ser::Error::custom)?;
        pending.push((self.root, None));

        let mut next_index = 0u32;
        while let Some((entry, parent_index)) = pending.pop() {
            let index = next_index;
            next_index = next_index
                .checked_add(1)
                .ok_or_else(|| serde::ser::Error::custom("cache node index overflow"))?;
            sequence.serialize_element(&FlatNodeRef {
                parent_index,
                name: &entry.name,
                path: &entry.path,
                size: entry.size,
                own_size: entry.own_size,
                is_dir: entry.is_dir,
                ext: &entry.ext,
                file_count: entry.file_count,
                dir_count: entry.dir_count,
                modified_time: entry.modified_time,
                lod_expand: entry.lod_expand.as_ref(),
            })?;

            pending
                .try_reserve(entry.children.len())
                .map_err(serde::ser::Error::custom)?;
            for child in entry.children.iter().rev() {
                pending.push((child, Some(index)));
            }
        }

        sequence.end()
    }
}

struct BoundedVecWriter {
    bytes: Vec<u8>,
    limit: usize,
}

impl BoundedVecWriter {
    fn new(limit: u64) -> anyhow::Result<Self> {
        let limit = usize::try_from(limit)
            .map_err(|_| anyhow::anyhow!("cache byte limit does not fit this platform"))?;
        Ok(Self {
            bytes: Vec::new(),
            limit,
        })
    }

    fn finish(self) -> Vec<u8> {
        self.bytes
    }
}

impl Write for BoundedVecWriter {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        let remaining = self.limit.saturating_sub(self.bytes.len());
        if buffer.len() > remaining {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("serialized cache exceeds {} byte safety limit", self.limit),
            ));
        }
        self.bytes
            .try_reserve(buffer.len())
            .map_err(|error| std::io::Error::other(format!("cache allocation failed: {error}")))?;
        self.bytes.extend_from_slice(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn decode_bounded<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> anyhow::Result<T> {
    bincode::DefaultOptions::new()
        .with_fixint_encoding()
        .with_little_endian()
        .reject_trailing_bytes()
        .with_limit(MAX_CACHE_BYTES)
        .deserialize(bytes)
        .map_err(Into::into)
}

fn load_cache(root: &ScanRoot) -> anyhow::Result<Option<PreparedCache>> {
    let Some(current_path) = cache_path(root) else {
        anyhow::bail!("could not determine cache directory");
    };
    let legacy_path = legacy_cache_path(root);
    let path = if current_path.exists() {
        current_path
    } else if let Some(path) = legacy_path.filter(|path| path.exists()) {
        path
    } else {
        debug!("No cache found for: {}", root.display());
        return Ok(None);
    };

    let metadata = fs::metadata(&path)?;
    if metadata.len() > MAX_CACHE_BYTES {
        return Err(discard_invalid_cache(
            &path,
            anyhow::anyhow!(
                "cache is {} bytes, exceeds {} byte safety limit",
                metadata.len(),
                MAX_CACHE_BYTES
            ),
        ));
    }
    let bytes = read_cache_bytes(&path, metadata.len())?;
    let version = cache_version(&bytes).map_err(|error| discard_invalid_cache(&path, error))?;
    if version < CACHE_VERSION {
        remove_obsolete_cache(&path, version)?;
        return Ok(None);
    }
    if version > CACHE_VERSION {
        anyhow::bail!(
            "cache {:?} uses newer version {version}; preserved for a newer application",
            path
        );
    }

    let cached =
        decode_flat_cache(root, &bytes).map_err(|error| discard_invalid_cache(&path, error))?;
    let ext_stats = crate::app::helpers::compute_ext_stats(&cached.tree);
    let size_range = crate::app::helpers::compute_size_range(&cached.tree);

    info!(
        "Cache loaded: {:?} ({} files)",
        path, cached.tree.file_count
    );
    Ok(Some(PreparedCache {
        cached,
        ext_stats,
        size_range,
    }))
}

fn read_cache_bytes(path: &Path, expected_len: u64) -> anyhow::Result<Vec<u8>> {
    let expected_len = usize::try_from(expected_len)
        .map_err(|_| anyhow::anyhow!("cache length does not fit this platform"))?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(expected_len)
        .map_err(|error| anyhow::anyhow!("cache allocation failed for {path:?}: {error}"))?;

    let file = fs::File::open(path)?;
    file.take(MAX_CACHE_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_CACHE_BYTES {
        anyhow::bail!("cache grew beyond {MAX_CACHE_BYTES} byte safety limit while reading");
    }
    Ok(bytes)
}

fn cache_version(bytes: &[u8]) -> anyhow::Result<u32> {
    let encoded = bytes
        .get(..std::mem::size_of::<u32>())
        .ok_or_else(|| anyhow::anyhow!("cache header is truncated"))?;
    Ok(u32::from_le_bytes(encoded.try_into().map_err(|_| {
        anyhow::anyhow!("cache version header has invalid width")
    })?))
}

fn remove_obsolete_cache(path: &Path, version: u32) -> anyhow::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => {
            info!(
                "Removed obsolete cache {:?} (version {version}); a fresh scan will create version {CACHE_VERSION}",
                path
            );
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(anyhow::anyhow!(
            "obsolete cache {:?} could not be removed: {error}",
            path
        )),
    }
}

fn decode_flat_cache(root: &ScanRoot, bytes: &[u8]) -> anyhow::Result<CachedScan> {
    let flat: FlatCachedScan = decode_bounded(bytes)?;
    if flat.version != CACHE_VERSION {
        anyhow::bail!("cache version {} is unsupported", flat.version);
    }
    if flat.root_id != root.id() {
        anyhow::bail!("cache root identity does not match requested root");
    }
    if flat.scan_path != root.display() {
        anyhow::bail!("cache display root does not match requested root");
    }
    let tree = inflate_flat_tree(flat.nodes, root.path())?;
    let cached = CachedScan {
        version: flat.version,
        root_id: flat.root_id,
        scan_path: flat.scan_path,
        timestamp: flat.timestamp,
        quality: flat.quality,
        tree,
    };
    validate_reusable(root, &cached)?;
    Ok(cached)
}

fn inflate_flat_tree(nodes: Vec<FlatNode>, root: &Path) -> anyhow::Result<DirEntry> {
    let child_counts = validate_flat_nodes(&nodes, root)?;
    let mut entries = Vec::new();
    entries
        .try_reserve_exact(nodes.len())
        .map_err(|error| anyhow::anyhow!("cache tree allocation failed: {error}"))?;

    for (node, child_count) in nodes.into_iter().zip(child_counts) {
        let mut children = Vec::new();
        children
            .try_reserve_exact(child_count)
            .map_err(|error| anyhow::anyhow!("cache child allocation failed: {error}"))?;
        entries.push(InflatingNode {
            parent_index: node.parent_index,
            entry: Some(DirEntry {
                name: node.name,
                path: node.path,
                size: node.size,
                own_size: node.own_size,
                children,
                is_dir: node.is_dir,
                ext: node.ext,
                file_count: node.file_count,
                dir_count: node.dir_count,
                modified_time: node.modified_time,
                rect: Cell::new([0.0; 4]),
                lod_expand: node.lod_expand,
            }),
        });
    }

    let mut tree = None;
    for index in (0..entries.len()).rev() {
        let parent_index = entries[index].parent_index;
        let mut entry = entries[index]
            .entry
            .take()
            .ok_or_else(|| anyhow::anyhow!("cache node {index} was assembled twice"))?;
        entry.children.reverse();
        if let Some(parent_index) = parent_index {
            let parent_index = parent_index as usize;
            let parent = entries[parent_index].entry.as_mut().ok_or_else(|| {
                anyhow::anyhow!("cache parent {parent_index} was assembled early")
            })?;
            parent.children.push(entry);
        } else {
            tree = Some(entry);
        }
    }

    tree.ok_or_else(|| anyhow::anyhow!("cache contains no root node"))
}

struct InflatingNode {
    parent_index: Option<u32>,
    entry: Option<DirEntry>,
}

#[derive(Debug, Clone, Copy)]
struct Aggregate {
    size: u64,
    files: u64,
    dirs: u64,
    is_dir: bool,
}

impl Aggregate {
    fn add_child(&mut self, child: Self, path: &Path) -> anyhow::Result<()> {
        self.size = self
            .size
            .checked_add(child.size)
            .ok_or_else(|| anyhow::anyhow!("cached size overflow at {path:?}"))?;
        self.files = self
            .files
            .checked_add(child.files)
            .ok_or_else(|| anyhow::anyhow!("cached file-count overflow at {path:?}"))?;
        self.dirs = self
            .dirs
            .checked_add(
                child
                    .dirs
                    .checked_add(u64::from(child.is_dir))
                    .ok_or_else(|| anyhow::anyhow!("cached dir-count overflow at {path:?}"))?,
            )
            .ok_or_else(|| anyhow::anyhow!("cached dir-count overflow at {path:?}"))?;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
struct FlatNodeState {
    aggregate: Aggregate,
    child_count: usize,
}

fn validate_flat_nodes(nodes: &[FlatNode], root: &Path) -> anyhow::Result<Vec<usize>> {
    if nodes.is_empty() {
        anyhow::bail!("cache contains no root node");
    }

    let mut states: Vec<FlatNodeState> = Vec::new();
    states
        .try_reserve_exact(nodes.len())
        .map_err(|error| anyhow::anyhow!("cache validation allocation failed: {error}"))?;

    for (index, node) in nodes.iter().enumerate() {
        if !node.path.starts_with(root) {
            anyhow::bail!("cache entry escapes scan root: {:?}", node.path);
        }
        match (index, node.parent_index) {
            (0, None) => {
                if node.path != root {
                    anyhow::bail!("cache tree root does not match requested root");
                }
            }
            (0, Some(_)) => anyhow::bail!("cache root has a parent"),
            (_, None) => anyhow::bail!("cache node {index} has no parent"),
            (_, Some(parent_index)) => {
                let parent_index = parent_index as usize;
                if parent_index >= index {
                    anyhow::bail!("cache node {index} has non-ancestor parent {parent_index}");
                }
                let parent = &nodes[parent_index];
                if !parent.is_dir {
                    anyhow::bail!("cache node {index} has file parent {parent_index}");
                }
                if node.path.parent() != Some(parent.path.as_path()) {
                    anyhow::bail!("invalid cached parent relation at {:?}", node.path);
                }
                states[parent_index].child_count = states[parent_index]
                    .child_count
                    .checked_add(1)
                    .ok_or_else(|| anyhow::anyhow!("cache child-count overflow"))?;
            }
        }
        validate_node_shape(
            &node.path,
            node.size,
            node.own_size,
            node.is_dir,
            node.file_count,
            node.dir_count,
            node.lod_expand.as_ref(),
        )?;
        states.push(FlatNodeState {
            aggregate: if node.is_dir {
                Aggregate {
                    size: node.own_size,
                    files: 0,
                    dirs: 0,
                    is_dir: true,
                }
            } else {
                Aggregate {
                    size: node.size,
                    files: node.file_count,
                    dirs: 0,
                    is_dir: false,
                }
            },
            child_count: 0,
        });
    }

    for index in (0..nodes.len()).rev() {
        let node = &nodes[index];
        let state = states[index];
        if !node.is_dir && state.child_count != 0 {
            anyhow::bail!("cached file has children at {:?}", node.path);
        }
        if (node.size, node.file_count, node.dir_count)
            != (
                state.aggregate.size,
                state.aggregate.files,
                state.aggregate.dirs,
            )
        {
            anyhow::bail!("cached aggregate mismatch at {:?}", node.path);
        }
        if let Some(parent_index) = node.parent_index {
            let child = state.aggregate;
            states[parent_index as usize]
                .aggregate
                .add_child(child, &nodes[parent_index as usize].path)?;
        }
    }

    Ok(states.into_iter().map(|state| state.child_count).collect())
}

fn validate_node_shape(
    path: &Path,
    size: u64,
    own_size: u64,
    is_dir: bool,
    file_count: u64,
    dir_count: u64,
    lod_expand: Option<&LodExpandInfo>,
) -> anyhow::Result<()> {
    if is_dir {
        if lod_expand.is_some() {
            anyhow::bail!("cached directory has LoD leaf metadata at {path:?}");
        }
        return Ok(());
    }
    if own_size != size || dir_count != 0 {
        anyhow::bail!("invalid cached file counters at {path:?}");
    }
    match lod_expand {
        Some(info) => {
            if file_count == 0 || path.parent() != Some(info.parent_dir.as_path()) {
                anyhow::bail!("invalid cached LoD leaf at {path:?}");
            }
        }
        None if file_count != 1 => {
            anyhow::bail!("invalid cached file count at {path:?}");
        }
        None => {}
    }
    Ok(())
}

fn discard_invalid_cache(path: &Path, error: anyhow::Error) -> anyhow::Error {
    match fs::remove_file(path) {
        Ok(()) => anyhow::anyhow!("invalid cache {:?}: {error:#}; removed", path),
        Err(cleanup) if cleanup.kind() == std::io::ErrorKind::NotFound => {
            anyhow::anyhow!("invalid cache {:?}: {error:#}; already absent", path)
        }
        Err(cleanup) => anyhow::anyhow!(
            "invalid cache {:?}: {error:#}; removal failed: {cleanup}",
            path
        ),
    }
}

fn validate_reusable(root: &ScanRoot, cached: &CachedScan) -> anyhow::Result<()> {
    validate_cached(root, cached)?;
    if !cached.quality.complete {
        anyhow::bail!("partial cache snapshots are not reusable");
    }
    Ok(())
}

fn validate_cached(root: &ScanRoot, cached: &CachedScan) -> anyhow::Result<()> {
    if cached.version != CACHE_VERSION {
        anyhow::bail!("cache version {} is unsupported", cached.version);
    }
    if cached.root_id != root.id() {
        anyhow::bail!("cache root identity does not match requested root");
    }
    if cached.scan_path != root.display() {
        anyhow::bail!("cache display root does not match requested root");
    }
    if cached.tree.path != root.path() {
        anyhow::bail!("cache tree root does not match requested root");
    }
    validate_tree(&cached.tree, root.path()).map(|_| ())
}

fn validate_tree(entry: &DirEntry, root: &Path) -> anyhow::Result<usize> {
    if entry.path != root {
        anyhow::bail!("cache tree root does not match requested root");
    }

    let mut pending = Vec::new();
    pending
        .try_reserve(1)
        .map_err(|error| anyhow::anyhow!("cache validation allocation failed: {error}"))?;
    pending.push((entry, false));
    let mut aggregates = Vec::new();
    let mut node_count = 0usize;

    while let Some((entry, visited)) = pending.pop() {
        if !visited {
            if !entry.path.starts_with(root) {
                anyhow::bail!("cache entry escapes scan root: {:?}", entry.path);
            }
            validate_node_shape(
                &entry.path,
                entry.size,
                entry.own_size,
                entry.is_dir,
                entry.file_count,
                entry.dir_count,
                entry.lod_expand.as_ref(),
            )?;
            if !entry.is_dir && !entry.children.is_empty() {
                anyhow::bail!("cached file has children at {:?}", entry.path);
            }

            node_count = node_count
                .checked_add(1)
                .ok_or_else(|| anyhow::anyhow!("cache node-count overflow"))?;
            if node_count > u32::MAX as usize {
                anyhow::bail!("cache contains more than {} nodes", u32::MAX);
            }

            pending
                .try_reserve(
                    entry
                        .children
                        .len()
                        .checked_add(1)
                        .ok_or_else(|| anyhow::anyhow!("cache traversal capacity overflow"))?,
                )
                .map_err(|error| anyhow::anyhow!("cache traversal allocation failed: {error}"))?;
            pending.push((entry, true));
            for child in entry.children.iter().rev() {
                if child.path.parent() != Some(entry.path.as_path()) {
                    anyhow::bail!("invalid cached parent relation at {:?}", child.path);
                }
                pending.push((child, false));
            }
            continue;
        }

        let mut aggregate = if entry.is_dir {
            Aggregate {
                size: entry.own_size,
                files: 0,
                dirs: 0,
                is_dir: true,
            }
        } else {
            Aggregate {
                size: entry.size,
                files: entry.file_count,
                dirs: 0,
                is_dir: false,
            }
        };
        for _ in 0..entry.children.len() {
            let child = aggregates
                .pop()
                .ok_or_else(|| anyhow::anyhow!("cache validation stack underflow"))?;
            aggregate.add_child(child, &entry.path)?;
        }
        if (entry.size, entry.file_count, entry.dir_count)
            != (aggregate.size, aggregate.files, aggregate.dirs)
        {
            anyhow::bail!("cached aggregate mismatch at {:?}", entry.path);
        }
        aggregates
            .try_reserve(1)
            .map_err(|error| anyhow::anyhow!("cache validation allocation failed: {error}"))?;
        aggregates.push(aggregate);
    }

    if aggregates.len() != 1 {
        anyhow::bail!("cache validation ended with {} roots", aggregates.len());
    }
    Ok(node_count)
}

fn write_cache_bytes(root: &ScanRoot, bytes: &[u8]) -> anyhow::Result<()> {
    let Some(cache_file) = cache_path(root) else {
        anyhow::bail!("could not determine cache directory");
    };
    let outcome = crate::atomic_file::write(&cache_file, bytes)?;
    if let Some(warning) = outcome.warning() {
        warn!("{warning}");
    }
    info!("Cache saved: {:?} ({} bytes)", cache_file, bytes.len());
    Ok(())
}

fn delete_cache(root: &ScanRoot) -> anyhow::Result<()> {
    for path in [cache_path(root), legacy_cache_path(root)]
        .into_iter()
        .flatten()
    {
        match fs::remove_file(&path) {
            Ok(()) => info!("Cache deleted: {:?}", path),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

pub fn age_secs_from_cached(cached: &CachedScan) -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs().saturating_sub(cached.timestamp))
        .unwrap_or(0)
}

pub fn format_age(seconds: u64) -> String {
    if seconds < 60 {
        format!("{seconds}s ago")
    } else if seconds < 3600 {
        format!("{}m ago", seconds / 60)
    } else if seconds < 86400 {
        format!("{}h ago", seconds / 3600)
    } else {
        format!("{}d ago", seconds / 86400)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_cached(root: &ScanRoot) -> CachedScan {
        CachedScan {
            version: CACHE_VERSION,
            root_id: root.id().to_owned(),
            scan_path: root.display().to_owned(),
            timestamp: 0,
            quality: CacheQuality {
                complete: true,
                errors: 0,
            },
            tree: DirEntry::new_dir("root".into(), root.path().to_path_buf()),
        }
    }

    #[test]
    fn rejects_tree_with_wrong_aggregate() {
        let root = ScanRoot::from_input(".").expect("current directory must resolve");
        let mut cached = valid_cached(&root);
        cached.tree.size = 1;
        assert!(validate_cached(&root, &cached).is_err());
    }

    #[test]
    fn rejects_envelope_for_another_root() {
        let root = ScanRoot::from_input(".").expect("current directory must resolve");
        let mut cached = valid_cached(&root);
        cached.root_id = "wrong".into();
        assert!(validate_cached(&root, &cached).is_err());
    }

    #[test]
    fn rejects_partial_snapshot() {
        let root = ScanRoot::from_input(".").expect("current directory must resolve");
        let mut cached = valid_cached(&root);
        cached.quality.complete = false;
        cached.quality.errors = 1;
        assert!(validate_reusable(&root, &cached).is_err());
    }

    #[test]
    fn flat_cache_round_trip_preserves_tree_order() {
        let root = ScanRoot::from_input(".").expect("current directory must resolve");
        let mut tree = DirEntry::new_dir("root".into(), root.path().to_path_buf());
        tree.children.push(DirEntry::new_file(
            "a.bin".into(),
            root.path().join("a.bin"),
            3,
            "bin".into(),
            Some(7),
        ));
        tree.children.push(DirEntry::new_file(
            "b.txt".into(),
            root.path().join("b.txt"),
            5,
            "txt".into(),
            Some(9),
        ));
        tree.size = 8;
        tree.file_count = 2;

        let bytes = serialize_cache(
            &root,
            &tree,
            CacheQuality {
                complete: true,
                errors: 0,
            },
        )
        .expect("serialize cache");
        let decoded = decode_flat_cache(&root, &bytes).expect("decode cache");

        assert_eq!(decoded.tree.size, 8);
        assert_eq!(decoded.tree.file_count, 2);
        assert_eq!(decoded.tree.children[0].name, "a.bin");
        assert_eq!(decoded.tree.children[1].name, "b.txt");
    }

    #[test]
    fn bounded_decoder_rejects_truncated_and_trailing_input() {
        let root = ScanRoot::from_input(".").expect("current directory must resolve");
        let mut bytes = serialize_cache(
            &root,
            &valid_cached(&root).tree,
            CacheQuality {
                complete: true,
                errors: 0,
            },
        )
        .expect("serialize cache");
        let mut truncated = bytes.clone();
        truncated.pop();
        assert!(decode_bounded::<FlatCachedScan>(&truncated).is_err());

        bytes.push(0);
        assert!(decode_bounded::<FlatCachedScan>(&bytes).is_err());
    }

    #[test]
    fn flat_cache_rejects_non_ancestor_parent() {
        let root = ScanRoot::from_input(".").expect("current directory must resolve");
        let mut tree = DirEntry::new_dir("root".into(), root.path().to_path_buf());
        tree.children.push(DirEntry::new_file(
            "file".into(),
            root.path().join("file"),
            1,
            String::new(),
            None,
        ));
        tree.size = 1;
        tree.file_count = 1;
        let bytes = serialize_cache(
            &root,
            &tree,
            CacheQuality {
                complete: true,
                errors: 0,
            },
        )
        .expect("serialize cache");
        let mut flat: FlatCachedScan = decode_bounded(&bytes).expect("decode flat cache");
        flat.nodes[1].parent_index = Some(1);

        assert!(validate_flat_nodes(&flat.nodes, root.path()).is_err());
    }

    #[test]
    fn deep_tree_round_trip_is_iterative() {
        let root = ScanRoot::from_input(".").expect("current directory must resolve");
        const DEPTH: usize = 2_048;

        let mut paths = Vec::new();
        let mut path = root.path().to_path_buf();
        for index in 0..DEPTH {
            path.push(format!("d{index}"));
            paths.push(path.clone());
        }
        let mut child =
            DirEntry::new_file("leaf".into(), path.join("leaf"), 1, String::new(), None);
        for path in paths.into_iter().rev() {
            let mut parent = DirEntry::new_dir(
                path.file_name()
                    .expect("directory name")
                    .to_string_lossy()
                    .into_owned(),
                path,
            );
            parent.size = child.size;
            parent.file_count = child.file_count;
            parent.dir_count = child.dir_count + u64::from(child.is_dir);
            parent.children.push(child);
            child = parent;
        }
        let mut tree = DirEntry::new_dir("root".into(), root.path().to_path_buf());
        tree.size = child.size;
        tree.file_count = child.file_count;
        tree.dir_count = child.dir_count + 1;
        tree.children.push(child);

        let bytes = serialize_cache(
            &root,
            &tree,
            CacheQuality {
                complete: true,
                errors: 0,
            },
        )
        .expect("serialize deep cache");
        let decoded = decode_flat_cache(&root, &bytes).expect("decode deep cache");
        assert_eq!(decoded.tree.file_count, 1);
        assert_eq!(
            decoded.tree.dir_count,
            u64::try_from(DEPTH).expect("depth fits u64")
        );
    }

    #[test]
    fn delete_tombstone_wins_same_generation_and_rejects_stale_commands() {
        let mut watermark = CacheWatermark::default();
        assert!(watermark.accept_store(2));
        assert!(watermark.accept_delete(2));
        assert!(!watermark.accept_store(2));
        assert!(!watermark.accept_store(1));
        assert!(!watermark.accept_delete(1));
        assert!(watermark.accept_store(3));
        assert!(watermark.accept_delete(3));
        assert!(!watermark.accept_store(3));
    }
}
