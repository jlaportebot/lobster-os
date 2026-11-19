//! In-memory [`Fs`] for host tests.

use super::fsops::Fs;
use super::model::Entry;
use super::paths;
use crate::fs::vfs::VfsError;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

/// In-memory filesystem; `None` marks a folder.
#[derive(Default)]
pub struct MemFs {
    pub nodes: BTreeMap<String, Option<Vec<u8>>>,
    pub fail_write_containing: Option<&'static str>,
    pub no_rename: bool,
}

impl MemFs {
    pub fn with(items: &[(&str, Option<&str>)]) -> Self {
        let mut fs = MemFs::default();
        fs.nodes.insert("/".into(), None);
        for (path, data) in items {
            fs.nodes
                .insert((*path).into(), data.map(|d| d.as_bytes().to_vec()));
        }
        fs
    }

    pub fn paths(&self) -> Vec<String> {
        self.nodes.keys().cloned().collect()
    }
}

impl Fs for MemFs {
    fn list(&self, dir: &str) -> Result<Vec<Entry>, VfsError> {
        if !matches!(self.nodes.get(dir), Some(None)) {
            return Err(VfsError::NotADirectory);
        }
        Ok(self
            .nodes
            .keys()
            .filter(|p| p.as_str() != dir && paths::parent(p) == dir)
            .filter_map(|p| self.stat(p))
            .collect())
    }

    fn stat(&self, path: &str) -> Option<Entry> {
        let node = self.nodes.get(path)?;
        Some(Entry {
            name: paths::file_name(path).into(),
            is_dir: node.is_none(),
            size: node.as_ref().map_or(0, |d| d.len() as u64),
            mtime: 0,
        })
    }

    fn read(&self, path: &str) -> Result<Vec<u8>, VfsError> {
        match self.nodes.get(path) {
            Some(Some(data)) => Ok(data.clone()),
            Some(None) => Err(VfsError::NotAFile),
            None => Err(VfsError::NotFound),
        }
    }

    fn write(&mut self, path: &str, data: &[u8]) -> Result<(), VfsError> {
        if self.fail_write_containing.is_some_and(|s| path.contains(s)) {
            return Err(VfsError::IOError);
        }
        self.nodes.insert(path.into(), Some(data.to_vec()));
        Ok(())
    }

    fn mkdir(&mut self, path: &str) -> Result<(), VfsError> {
        self.nodes.insert(path.into(), None);
        Ok(())
    }

    fn remove(&mut self, path: &str) -> Result<(), VfsError> {
        self.nodes
            .remove(path)
            .map(|_| ())
            .ok_or(VfsError::NotFound)
    }

    fn rename(&mut self, from: &str, to: &str) -> Result<(), VfsError> {
        if self.no_rename {
            return Err(VfsError::Busy);
        }
        let moved: Vec<String> = self
            .nodes
            .keys()
            .filter(|p| paths::is_same_or_inside(p, from))
            .cloned()
            .collect();
        for old in moved {
            let node = self.nodes.remove(&old).unwrap_or(None);
            let new = format!("{}{}", to, &old[from.len()..]);
            self.nodes.insert(new, node);
        }
        Ok(())
    }
}
