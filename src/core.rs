use crate::{
    filter::FilterParseError,
    read::{get_store_path, read_store_file, DirData, FileData, GlobMatches},
    walk::WalkDirectories,
};
use glob_match::glob_match;
use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
};

pub(crate) const FSTORE: &str = ".fstore";

#[derive(Debug)]
pub(crate) enum FstoreError {
    InvalidCommand(String),
    InteractiveModeError(String),
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

pub(crate) fn check(path: PathBuf) -> Result<(), FstoreError> {
    let mut success = true;
    let mut walker = WalkDirectories::from(path)?;
    let mut matcher = GlobMatches::new();
    while let Some((_depth, dirpath, children)) = walker.next() {
        let DirData {
            files,
            desc: _,
            tags: _,
        } = {
            match get_store_path::<true>(&dirpath) {
                Some(path) => read_store_file(path)?,
                None => continue,
            }
        };
        if let Some(files) = files {
            matcher.find_matches(children, &files, true);
            for pattern in files.iter().enumerate().filter_map(|(i, f)| {
                if !matcher.is_glob_matched(i) {
                    Some(&f.path)
                } else {
                    None
                }
            }) {
                eprintln!("No files matching '{}' in {}", pattern, dirpath.display());
                success = false;
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

fn full_description(tags: Vec<String>, desc: String) -> String {
    let tagstr = {
        let mut tags = tags.into_iter();
        let first = tags.next().unwrap_or(String::new());
        tags.fold(first, |acc, t| format!("{}, {}", acc, t))
    };
    format!(
        "tags: [{}]{}",
        tagstr,
        if desc.is_empty() {
            desc
        } else {
            format!("\n\n{}", desc)
        }
    )
}

pub(crate) fn what_is(path: &PathBuf) -> Result<String, FstoreError> {
    if path.is_file() {
        what_is_file(path)
    } else if path.is_dir() {
        what_is_dir(path)
    } else {
        Err(FstoreError::InvalidPath(path.clone()))
    }
}

fn what_is_file(path: &PathBuf) -> Result<String, FstoreError> {
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
    return Ok(full_description(outtags, outdesc));
}

fn what_is_dir(path: &PathBuf) -> Result<String, FstoreError> {
    let DirData {
        desc,
        tags,
        files: _,
    } = {
        match get_store_path::<true>(path) {
            Some(storepath) => read_store_file(storepath)?,
            None => return Err(FstoreError::InvalidPath(path.clone())),
        }
    };
    let desc = desc.unwrap_or(String::new());
    let tags = tags.unwrap_or(Vec::new());
    return Ok(full_description(tags, desc));
}

pub(crate) fn get_relative_path(dirpath: &Path, filename: &OsStr, root: &Path) -> Option<PathBuf> {
    match dirpath.strip_prefix(root) {
        Ok(path) => {
            let mut path = PathBuf::from(path);
            path.push(filename);
            Some(path)
        }
        Err(_) => None,
    }
}

pub(crate) fn untracked_files(root: PathBuf) -> Result<Vec<PathBuf>, FstoreError> {
    let mut walker = WalkDirectories::from(root.clone())?;
    let mut untracked: Vec<PathBuf> = Vec::new();
    while let Some((_depth, dirpath, children)) = walker.next() {
        let DirData {
            files,
            desc: _,
            tags: _,
        }: DirData = {
            match get_store_path::<true>(&dirpath) {
                Some(path) => read_store_file(path)?,
                // Store file doesn't exist so everything is untracked.
                None => {
                    untracked.extend(
                        children
                            .iter()
                            .filter_map(|f| get_relative_path(&dirpath, &f.name(), &root)),
                    );
                    continue;
                }
            }
        };
        if let Some(patterns) = files {
            untracked.extend(children.iter().filter_map(|child| {
                let fnamestr = child.name().to_str()?;
                if patterns.iter().any(|p| glob_match(&p.path, fnamestr)) {
                    None
                } else {
                    get_relative_path(&dirpath, &child.name(), &root)
                }
            }));
        } else {
            untracked.extend(
                children
                    .iter()
                    .filter_map(|child| get_relative_path(&dirpath, &child.name(), &root)),
            );
        }
    }
    return Ok(untracked);
}

pub(crate) fn get_all_tags(path: PathBuf) -> Result<Vec<String>, FstoreError> {
    let mut alltags: Vec<String> = Vec::new();
    let mut walker = WalkDirectories::from(path)?;
    while let Some((_depth, dirpath, _filenames)) = walker.next() {
        let DirData {
            tags,
            files,
            desc: _,
        } = {
            match get_store_path::<true>(&dirpath) {
                Some(path) => read_store_file(path)?,
                None => continue,
            }
        };
        if let Some(mut tags) = tags {
            alltags.extend(tags.drain(..));
        }
        if let Some(mut files) = files {
            for fdata in files.drain(..) {
                if let Some(mut ftags) = fdata.tags {
                    alltags.extend(ftags.drain(..));
                }
            }
        }
    }
    alltags.sort();
    alltags.dedup();
    return Ok(alltags);
}
