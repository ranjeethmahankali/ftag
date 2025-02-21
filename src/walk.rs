use std::{
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
};

use crate::{
    core::{Error, FTAG_BACKUP_FILE, FTAG_FILE},
    load::{DirData, Loader, LoaderOptions, get_ftag_path},
};

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
pub(crate) struct DirTree {
    abs_dir_path: PathBuf,
    rel_dir_path: PathBuf,
    stack: Vec<DirEntry>,
    cur_depth: usize,
    num_children: usize,
    loader: Loader,
}

pub(crate) enum MetaData<'a> {
    Ok(&'a DirData<'a>),
    NotFound,
    FailedToLoad(Error),
}

pub(crate) struct VisitedDir<'a> {
    pub(crate) traverse_depth: usize,
    pub(crate) abs_dir_path: &'a Path,
    pub(crate) rel_dir_path: &'a Path,
    pub(crate) files: &'a [DirEntry],
    pub(crate) metadata: MetaData<'a>,
}

fn is_ftag_file(file: &OsStr) -> bool {
    file == OsStr::new(FTAG_FILE) || file == OsStr::new(FTAG_BACKUP_FILE)
}

impl DirTree {
    pub fn new(rootdir: PathBuf, options: LoaderOptions) -> Result<Self, Error> {
        if !rootdir.is_dir() {
            return Err(Error::InvalidPath(rootdir));
        }
        let mut stack = Vec::with_capacity(32);
        stack.push(DirEntry {
            depth: 1,
            entry_type: DirEntryType::Dir,
            name: OsString::new(),
        });
        Ok(DirTree {
            abs_dir_path: rootdir,
            rel_dir_path: PathBuf::new(),
            stack,
            cur_depth: 0,
            num_children: 0,
            loader: Loader::new(options),
        })
    }

    /// Move on to the next directory. Returns a tuple containing the depth of
    /// the directory, its absolute path, its path relative to the root of the
    /// walk, and a slice containing info about the files in this directory.
    pub fn walk(&mut self) -> Option<VisitedDir> {
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
                        self.abs_dir_path.pop();
                        self.rel_dir_path.pop();
                        self.cur_depth -= 1;
                    }
                    self.abs_dir_path.push(name.clone());
                    self.rel_dir_path.push(name);
                    self.cur_depth += 1;
                    // Push all children.
                    let mut numfiles = 0;
                    let before = self.stack.len();
                    if let Ok(entries) = std::fs::read_dir(&self.abs_dir_path) {
                        for child in entries.flatten() {
                            match (child.file_name(), child.file_type()) {
                                (cname, _) if is_ftag_file(&cname) => continue,
                                (cname, Ok(ctype)) if ctype.is_dir() => self.stack.push(DirEntry {
                                    depth: depth + 1,
                                    entry_type: DirEntryType::Dir,
                                    name: cname,
                                }),
                                (cname, Ok(ctype)) if ctype.is_file() => {
                                    self.stack.push(DirEntry {
                                        depth: depth + 1,
                                        entry_type: DirEntryType::File,
                                        name: cname,
                                    });
                                    numfiles += 1;
                                }
                                _ => continue,
                            }
                        }
                    }
                    self.num_children = self.stack.len() - before;
                    // Sort the contents of this folder to move all the files to the end of the stack.
                    self.stack[before..].sort_unstable_by(|a, b| {
                        match (a.entry_type, b.entry_type) {
                            (DirEntryType::File, DirEntryType::File) => a.name.cmp(&b.name),
                            (DirEntryType::File, DirEntryType::Dir) => std::cmp::Ordering::Greater,
                            (DirEntryType::Dir, DirEntryType::File) => std::cmp::Ordering::Less,
                            (DirEntryType::Dir, DirEntryType::Dir) => std::cmp::Ordering::Equal,
                        }
                    });
                    return Some(VisitedDir {
                        traverse_depth: depth,
                        abs_dir_path: &self.abs_dir_path,
                        rel_dir_path: &self.rel_dir_path,
                        files: &self.stack[(self.stack.len() - numfiles)..], // Files are sorted to the end of the stack.
                        metadata: match get_ftag_path::<true>(&self.abs_dir_path) {
                            Some(fpath) => match self.loader.load(&fpath) {
                                Ok(data) => MetaData::Ok(data),
                                Err(e) => MetaData::FailedToLoad(e),
                            },
                            None => MetaData::NotFound,
                        },
                    });
                }
            }
        }
        None
    }
}
