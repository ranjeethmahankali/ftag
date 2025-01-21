use std::{
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
};

use crate::core::{Error, FTAG_BACKUP_FILE, FTAG_FILE};

#[derive(PartialEq, Eq, Copy, Clone)]
pub(crate) enum DirEntryType {
    File,
    Dir,
}

/// Entry found during recursive traversal. `depth` 1 corresponds to
/// the root of the recursive traversal, and subsequent depths
/// indicate the level of nesting.
pub(crate) struct DirEntry {
    depth: usize,
    entry_type: DirEntryType,
    name: OsString,
}

impl DirEntry {
    pub fn name(&self) -> &OsStr {
        &self.name
    }
}

/// Recursively walk directories, while caching useful information
/// about the contents of the directory. The traversal is depth first.
pub(crate) struct WalkDirectories {
    cur_path: PathBuf,
    stack: Vec<DirEntry>,
    cur_depth: usize,
    num_children: usize,
}

impl WalkDirectories {
    pub fn from(dirpath: PathBuf) -> Result<Self, Error> {
        if !dirpath.is_dir() {
            return Err(Error::InvalidPath(dirpath));
        }
        Ok(WalkDirectories {
            cur_path: dirpath,
            stack: vec![DirEntry {
                depth: 1,
                entry_type: DirEntryType::Dir,
                name: OsString::from(""),
            }],
            cur_depth: 0,
            num_children: 0,
        })
    }

    /// Move on to the next directory. Returns a tuple containing the
    /// depth of the directory, its path, and a slice containing info
    /// about the files in this directory.
    pub(crate) fn next(&mut self) -> Option<(usize, &Path, &[DirEntry])> {
        while let Some(DirEntry {
            depth,
            entry_type,
            name,
        }) = self.stack.pop()
        {
            match entry_type {
                DirEntryType::File => continue,
                DirEntryType::Dir => {
                    while self.cur_depth > depth - 1 {
                        self.cur_path.pop();
                        self.cur_depth -= 1;
                    }
                    self.cur_path.push(name);
                    self.cur_depth += 1;
                    // Push all children.
                    let mut numfiles = 0;
                    let before = self.stack.len();
                    if let Ok(entries) = std::fs::read_dir(&self.cur_path) {
                        for child in entries.flatten() {
                            let cname = child.file_name();
                            if cname == OsStr::new(FTAG_FILE)
                                || cname == OsStr::new(FTAG_BACKUP_FILE)
                            {
                                continue;
                            }
                            match child.file_type() {
                                Ok(ctype) => {
                                    if ctype.is_dir() {
                                        self.stack.push(DirEntry {
                                            depth: depth + 1,
                                            entry_type: DirEntryType::Dir,
                                            name: cname,
                                        });
                                    } else if ctype.is_file() {
                                        self.stack.push(DirEntry {
                                            depth: depth + 1,
                                            entry_type: DirEntryType::File,
                                            name: cname,
                                        });
                                        numfiles += 1;
                                    }
                                }
                                Err(_) => continue,
                            }
                        }
                    }
                    self.num_children = self.stack.len() - before;
                    let children = &mut self.stack[before..];
                    children.sort_by(|a, b| match (a.entry_type, b.entry_type) {
                        (DirEntryType::File, DirEntryType::File) => a.name.cmp(&b.name),
                        (DirEntryType::File, DirEntryType::Dir) => std::cmp::Ordering::Greater,
                        (DirEntryType::Dir, DirEntryType::File) => std::cmp::Ordering::Less,
                        (DirEntryType::Dir, DirEntryType::Dir) => std::cmp::Ordering::Equal,
                    });
                    let children = &self.stack[(self.stack.len() - numfiles)..];
                    return Some((depth, &self.cur_path, children));
                }
            }
        }
        None
    }
}
