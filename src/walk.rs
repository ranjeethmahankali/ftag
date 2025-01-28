use std::{
    ffi::{OsStr, OsString},
    marker::PhantomData,
    path::{Path, PathBuf},
    ptr::NonNull,
};

use crate::{
    core::{Error, FTAG_BACKUP_FILE, FTAG_FILE},
    load::{get_ftag_path, DirData, Loader},
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

pub(crate) struct VisitedDir<'a> {
    pub(crate) traverse_depth: usize,
    pub(crate) abs_dir_path: &'a Path,
    pub(crate) rel_dir_path: &'a Path,
    pub(crate) files: &'a [DirEntry],
    pub(crate) metadata: Option<Result<DirData<'a>, Error>>,
}

pub(crate) struct DirIter<'a> {
    ptr: NonNull<DirTree>,
    phantom: PhantomData<&'a DirTree>,
}

impl<'a> Iterator for DirIter<'a> {
    type Item = VisitedDir<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let walker = unsafe { self.ptr.as_mut() };
        walker.next()
    }
}

impl DirTree {
    pub fn new(rootdir: PathBuf, loader: Loader) -> Result<Self, Error> {
        if !rootdir.is_dir() {
            return Err(Error::InvalidPath(rootdir));
        }
        Ok(DirTree {
            abs_dir_path: rootdir,
            rel_dir_path: PathBuf::new(),
            stack: vec![DirEntry {
                depth: 1,
                entry_type: DirEntryType::Dir,
                name: OsString::from(""),
            }],
            cur_depth: 0,
            num_children: 0,
            loader,
        })
    }

    pub fn walk<'a>(&'a mut self) -> DirIter<'a> {
        DirIter {
            ptr: NonNull::from(self),
            phantom: PhantomData,
        }
    }

    /// Move on to the next directory. Returns a tuple containing the depth of
    /// the directory, its absolute path, its path relative to the root of the
    /// walk, and a slice containing info about the files in this directory.
    fn next(&mut self) -> Option<VisitedDir> {
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
                    let metadata = get_ftag_path::<true>(&self.abs_dir_path)
                        .map(|path| self.loader.load(&path));
                    return Some(VisitedDir {
                        traverse_depth: depth,
                        abs_dir_path: &self.abs_dir_path,
                        rel_dir_path: &self.rel_dir_path,
                        files: children,
                        metadata,
                    });
                }
            }
        }
        None
    }
}
