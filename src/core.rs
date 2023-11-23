use crate::filter::FilterParseError;
use glob_match::glob_match;
use serde::{de::DeserializeOwned, Deserialize};
use std::{
    ffi::OsString,
    fs::File,
    io::BufReader,
    path::{Path, PathBuf},
};

#[derive(Debug)]
pub enum FstoreError {
    EditCommandFailed(String),
    MissingFiles,
    InvalidArgs,
    InvalidWorkingDirectory,
    InvalidPath(PathBuf),
    CannotReadStoreFile(PathBuf),
    CannotParseYaml(String),
    InvalidFilter(FilterParseError),
    DirectoryTraversalFailed,
    TagInheritanceFailed,
}

pub struct Info {
    pub tags: Vec<String>,
    pub desc: String,
}

pub(crate) fn glob_filter<'a>(pattern: &'a str) -> impl FnMut(&&'a OsString) -> bool {
    let func = |fname: &&OsString| -> bool {
        if let Some(fname) = fname.to_str() {
            return glob_match(pattern, fname);
        }
        return false;
    };
    return func;
}

pub(crate) enum DirEntryType {
    File,
    Dir,
}

pub struct DirEntry {
    depth: usize,
    entry_type: DirEntryType,
    name: OsString,
}

pub(crate) fn get_filenames<'a>(entries: &'a [DirEntry]) -> impl Iterator<Item = &'a OsString> {
    entries.iter().filter_map(|entry| match entry.entry_type {
        DirEntryType::File => Some(&entry.name),
        DirEntryType::Dir => None,
    })
}

pub struct WalkDirectories {
    cur_path: PathBuf,
    stack: Vec<DirEntry>,
    cur_depth: usize,
    num_children: usize,
}

impl WalkDirectories {
    pub fn from(dirpath: PathBuf) -> Result<Self, FstoreError> {
        if !dirpath.is_dir() {
            return Err(FstoreError::InvalidPath(dirpath));
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

    pub(crate) fn next<'a>(&'a mut self) -> Option<(usize, &'a Path, &'a [DirEntry])> {
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
                    let before = self.stack.len();
                    if let Ok(entries) = std::fs::read_dir(&self.cur_path) {
                        for entry in entries {
                            if let Ok(child) = entry {
                                let cname = child.file_name();
                                if cname.to_str().unwrap_or("") == FSTORE {
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
                                        }
                                    }
                                    Err(_) => continue,
                                }
                            }
                        }
                    }
                    self.num_children = self.stack.len() - before;
                    return Some((
                        depth,
                        &self.cur_path,
                        &self.stack[(self.stack.len() - self.num_children)..],
                    ));
                }
            }
        }
        return None;
    }
}

pub(crate) const FSTORE: &str = ".fstore";

pub(crate) fn get_store_path<const MUST_EXIST: bool>(path: &Path) -> Option<PathBuf> {
    let mut out = if path.exists() {
        if path.is_dir() {
            PathBuf::from(path)
        } else {
            let mut out = PathBuf::from(path);
            out.pop();
            out
        }
    } else {
        return None;
    };
    out.push(FSTORE);
    if MUST_EXIST && !out.exists() {
        None
    } else {
        Some(out)
    }
}

pub(crate) fn read_store_file<T: DeserializeOwned>(storefile: PathBuf) -> Result<T, FstoreError> {
    let data = serde_yaml::from_reader(BufReader::new(
        File::open(&storefile).map_err(|_| FstoreError::CannotReadStoreFile(storefile.clone()))?,
    ))
    .map_err(|e| FstoreError::CannotParseYaml(format!("{:?}\n{:?}", storefile, e)))?;
    return Ok(data);
}

pub fn check(path: PathBuf) -> Result<(), FstoreError> {
    #[derive(Deserialize)]
    struct FileData {
        path: String,
    }
    #[derive(Deserialize)]
    struct DirData {
        files: Option<Vec<FileData>>,
    }
    let mut success = true;
    let mut walker = WalkDirectories::from(path)?;
    while let Some((_depth, dirpath, children)) = walker.next() {
        let DirData { files } = {
            match get_store_path::<true>(&dirpath) {
                Some(path) => read_store_file(path)?,
                None => continue,
            }
        };
        if let Some(mut files) = files {
            for pattern in files.drain(..).map(|f| f.path) {
                if let None = get_filenames(children).filter(glob_filter(&pattern)).next() {
                    // Glob didn't match with any file.
                    eprintln!("No files matching '{}' in {}", pattern, dirpath.display());
                    success = false;
                }
            }
        }
    }
    if success {
        println!("No problems found.");
        Ok(())
    } else {
        Err(FstoreError::MissingFiles)
    }
}

pub fn what_is(path: &PathBuf) -> Result<Info, FstoreError> {
    if path.is_file() {
        what_is_file(path)
    } else if path.is_dir() {
        what_is_dir(path)
    } else {
        Err(FstoreError::InvalidPath(path.clone()))
    }
}

fn what_is_file(path: &PathBuf) -> Result<Info, FstoreError> {
    #[derive(Deserialize)]
    struct FileData {
        path: String,
        desc: Option<String>,
        tags: Option<Vec<String>>,
    }
    #[derive(Deserialize)]
    struct DirData {
        desc: Option<String>,
        tags: Option<Vec<String>>,
        files: Option<Vec<FileData>>,
    }
    let DirData { desc, tags, files } = {
        match get_store_path::<true>(path) {
            Some(storepath) => read_store_file(storepath)?,
            None => return Err(FstoreError::InvalidPath(path.clone())),
        }
    };
    let mut outdesc = desc.unwrap_or(String::new());
    let mut outtags = tags.unwrap_or(Vec::new());
    let filenamestr = path
        .file_name()
        .ok_or(FstoreError::InvalidPath(path.clone()))?
        .to_str()
        .ok_or(FstoreError::InvalidPath(path.clone()))?;
    if let Some(files) = files {
        for FileData {
            path: pattern,
            desc: fdesc,
            tags: ftags,
        } in files
        {
            if glob_match(&pattern, filenamestr) {
                if let Some(ftags) = ftags {
                    outtags.extend(ftags.into_iter());
                }
                if let Some(fdesc) = fdesc {
                    outdesc = format!("{}\n{}", fdesc, outdesc);
                }
            }
        }
    }
    // Remove duplicate tags.
    outtags.sort();
    outtags.dedup();
    return Ok(Info {
        tags: outtags,
        desc: outdesc,
    });
}

fn what_is_dir(path: &PathBuf) -> Result<Info, FstoreError> {
    #[derive(Deserialize)]
    struct DirData {
        desc: Option<String>,
        tags: Option<Vec<String>>,
    }
    let DirData { desc, tags } = {
        match get_store_path::<true>(path) {
            Some(storepath) => read_store_file(storepath)?,
            None => return Err(FstoreError::InvalidPath(path.clone())),
        }
    };
    let desc = desc.unwrap_or(String::new());
    let tags = tags.unwrap_or(Vec::new());
    Ok(Info { desc, tags })
}

pub(crate) fn get_relative_path(
    dirpath: &Path,
    filename: &OsString,
    root: &Path,
) -> Option<PathBuf> {
    match dirpath.strip_prefix(root) {
        Ok(path) => {
            let mut path = PathBuf::from(path);
            path.push(filename);
            Some(path)
        }
        Err(_) => None,
    }
}

pub fn untracked_files(root: PathBuf) -> Result<Vec<PathBuf>, FstoreError> {
    #[derive(Deserialize)]
    struct FileData {
        path: String,
    }
    #[derive(Deserialize)]
    struct DirData {
        files: Option<Vec<FileData>>,
    }
    let mut walker = WalkDirectories::from(root.clone())?;
    let mut untracked: Vec<PathBuf> = Vec::new();
    while let Some((_depth, dirpath, children)) = walker.next() {
        let DirData { files } = {
            match get_store_path::<true>(&dirpath) {
                Some(path) => read_store_file(path)?,
                // Store file doesn't exist so everything is untracked.
                None => {
                    untracked.extend(
                        get_filenames(children)
                            .filter_map(|f| get_relative_path(&dirpath, f, &root)),
                    );
                    continue;
                }
            }
        };
        if let Some(patterns) = files {
            untracked.extend(get_filenames(children).filter_map(|fname| {
                let fnamestr = fname.to_str()?;
                if patterns.iter().any(|p| glob_match(&p.path, fnamestr)) {
                    None
                } else {
                    get_relative_path(&dirpath, fname, &root)
                }
            }));
        } else {
            untracked.extend(
                get_filenames(children).filter_map(|f| get_relative_path(&dirpath, f, &root)),
            );
        }
    }
    return Ok(untracked);
}

pub fn get_all_tags(_path: PathBuf) -> Result<Vec<String>, FstoreError> {
    #[derive(Deserialize)]
    struct FileData {
        tags: Option<Vec<String>>,
    }
    #[derive(Deserialize)]
    struct DirData {
        tags: Option<Vec<String>>,
        files: Option<Vec<FileData>>,
    }
    let mut alltags: Vec<String> = Vec::new();
    let mut walker = WalkDirectories::from(_path)?;
    while let Some((_depth, dirpath, _filenames)) = walker.next() {
        let DirData { tags, files } = {
            match get_store_path::<true>(&dirpath) {
                Some(path) => read_store_file(path)?,
                None => continue,
            }
        };
        if let Some(mut tags) = tags {
            alltags.extend(tags.drain(..));
        }
        if let Some(mut files) = files {
            for ftags in files.drain(..) {
                if let Some(mut ftags) = ftags.tags {
                    alltags.extend(ftags.drain(..));
                }
            }
        }
    }
    alltags.sort();
    alltags.dedup();
    return Ok(alltags);
}
