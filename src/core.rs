use crate::{
    filter::FilterParseError,
    query::{GlobExpander, GlobExpansion},
};
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

pub struct WalkDirectories<const WITH_FILES: bool> {
    stack: Vec<PathBuf>,
    files: Vec<OsString>,
}

impl<const WITH_FILES: bool> WalkDirectories<WITH_FILES> {
    pub fn from(dirpath: PathBuf) -> Result<Self, FstoreError> {
        if !dirpath.is_dir() {
            return Err(FstoreError::InvalidPath(dirpath));
        }
        Ok(WalkDirectories {
            stack: vec![dirpath],
            files: Vec::new(),
        })
    }
}

impl WalkDirectories<true> {
    pub fn files(&self) -> &Vec<OsString> {
        &self.files
    }
}

impl<const WITH_FILES: bool> Iterator for WalkDirectories<WITH_FILES> {
    type Item = PathBuf;

    fn next(&mut self) -> Option<Self::Item> {
        match self.stack.pop() {
            Some(dir) => {
                self.files.clear();
                // Push all nested directories.
                if let Ok(entries) = std::fs::read_dir(&dir) {
                    for entry in entries {
                        if let Ok(child) = entry {
                            if child.file_name().to_str().unwrap_or("") == FSTORE {
                                continue;
                            }
                            match child.file_type() {
                                Ok(ctype) => {
                                    if ctype.is_dir() {
                                        self.stack.push(child.path());
                                    } else if WITH_FILES && ctype.is_file() {
                                        self.files.push(child.file_name());
                                    }
                                }
                                Err(_) => continue,
                            }
                        }
                    }
                }
                Some(dir)
            }
            None => None,
        }
    }
}

pub(crate) const FSTORE: &str = ".fstore";

pub fn get_store_path<const MUST_EXIST: bool>(path: &PathBuf) -> Option<PathBuf> {
    let mut out = if path.exists() {
        if path.is_dir() {
            path.clone()
        } else {
            let mut out = path.clone();
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

pub fn read_store_file<T: DeserializeOwned>(storefile: PathBuf) -> Result<T, FstoreError> {
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
    let mut expander = GlobExpander::new();
    let mut walker = WalkDirectories::<true>::from(path)?;
    while let Some(dirpath) = walker.next() {
        let DirData { files } = {
            match get_store_path::<true>(&dirpath) {
                Some(path) => read_store_file(path)?,
                None => continue,
            }
        };
        expander.init(dirpath.clone());
        if let Some(mut files) = files {
            for pattern in files.drain(..).map(|f| f.path) {
                match expander.expand(pattern.clone(), walker.files())? {
                    // One file is only returned when it exists.
                    GlobExpansion::One(file) => assert!(file.exists()),
                    // When a pattern doesn't exist, it is interpreted as a glob.
                    GlobExpansion::Many(mut iter) => {
                        if !iter.any(|file| file.exists()) {
                            // Glob didn't match with any file.
                            eprintln!("No files matching '{}' in {}", pattern, dirpath.display());
                            success = false;
                        }
                    }
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

fn get_relative_path(dirpath: &Path, filename: OsString, root: &Path) -> Option<PathBuf> {
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
    let mut walker = WalkDirectories::<true>::from(root.clone())?;
    let mut untracked: Vec<PathBuf> = Vec::new();
    while let Some(dirpath) = walker.next() {
        let DirData { files } = {
            match get_store_path::<true>(&dirpath) {
                Some(path) => read_store_file(path)?,
                // Store file doesn't exist so everything is untracked.
                None => {
                    untracked.extend(
                        walker
                            .files()
                            .iter()
                            .filter_map(|f| get_relative_path(&dirpath, f.clone(), &root)),
                    );
                    continue;
                }
            }
        };
        if let Some(patterns) = files {
            untracked.extend(
                walker
                    .files()
                    .iter()
                    .filter_map(|fname| match fname.to_str() {
                        Some(fstr) => {
                            if patterns.iter().any(|p| glob_match(&p.path, fstr)) {
                                None
                            } else {
                                Some(fname.clone())
                            }
                        }
                        None => Some(fname.clone()),
                    })
                    .filter_map(|fname| get_relative_path(&dirpath, fname, &root)),
            );
        } else {
            untracked.extend(
                walker
                    .files()
                    .iter()
                    .filter_map(|f| get_relative_path(&dirpath, f.clone(), &root)),
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
    for dirpath in WalkDirectories::<false>::from(_path)? {
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
