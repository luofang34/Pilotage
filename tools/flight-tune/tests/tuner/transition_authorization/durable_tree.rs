use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, PartialEq, Eq)]
pub(super) struct DurableTreeSnapshot {
    directories: BTreeSet<PathBuf>,
    files: BTreeMap<PathBuf, Vec<u8>>,
}

impl DurableTreeSnapshot {
    pub(super) fn capture(root: &Path) -> Self {
        let mut snapshot = Self {
            directories: BTreeSet::new(),
            files: BTreeMap::new(),
        };
        snapshot.capture_directory(root, root);
        snapshot
    }

    pub(super) fn assert_unchanged(&self, root: &Path) {
        assert_eq!(&Self::capture(root), self);
    }

    fn capture_directory(&mut self, root: &Path, directory: &Path) {
        for entry in fs::read_dir(directory).expect("read durable tree") {
            let entry = entry.expect("read durable tree entry");
            let path = entry.path();
            let relative = path
                .strip_prefix(root)
                .expect("make durable path relative")
                .to_path_buf();
            if path.is_dir() {
                self.directories.insert(relative);
                self.capture_directory(root, &path);
            } else {
                self.files
                    .insert(relative, fs::read(&path).expect("read durable object"));
            }
        }
    }
}
