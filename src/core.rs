use crate::{
    filter::FilterParseError,
    load::{
        get_store_path, implicit_tags_os_str, implicit_tags_str, DirData, FileData,
        FileLoadingOptions, GlobMatches, Loader, LoaderOptions,
    },
    walk::WalkDirectories,
};
use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
};

pub(crate) const FSTORE: &str = ".fstore";

#[derive(Debug)]
pub(crate) enum Error {
    InvalidCommand(String),
    InteractiveModeError(String),
    EditCommandFailed(String),
    UnmatchedPatterns,
    InvalidArgs,
    InvalidWorkingDirectory,
    InvalidPath(PathBuf),
    CannotReadStoreFile(PathBuf),
    CannotParseYaml(String),
    InvalidFilter(FilterParseError),
    DirectoryTraversalFailed,
    TagInheritanceFailed,
}

pub(crate) fn check(path: PathBuf) -> Result<(), Error> {
    let mut success = true;
    let mut walker = WalkDirectories::from(path)?;
    let mut matcher = GlobMatches::new();
    let mut loader = Loader::new(LoaderOptions::new(
        false,
        false,
        FileLoadingOptions::Load {
            file_tags: false,
            file_desc: false,
        },
    ));
    while let Some((_depth, dirpath, children)) = walker.next() {
        let DirData {
            files,
            desc: _,
            tags: _,
        } = {
            match get_store_path::<true>(&dirpath) {
                Some(path) => loader.load(&path)?,
                None => continue,
            }
        };
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
    if success {
        println!("No problems found.");
        Ok(())
    } else {
        Err(Error::UnmatchedPatterns)
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

pub(crate) fn what_is(path: &PathBuf) -> Result<String, Error> {
    if path.is_file() {
        what_is_file(path)
    } else if path.is_dir() {
        what_is_dir(path)
    } else {
        Err(Error::InvalidPath(path.clone()))
    }
}

fn what_is_file(path: &PathBuf) -> Result<String, Error> {
    use glob_match::glob_match;
    let mut loader = Loader::new(LoaderOptions::new(
        true,
        true,
        FileLoadingOptions::Load {
            file_tags: true,
            file_desc: true,
        },
    ));
    let DirData { desc, tags, files } = {
        match get_store_path::<true>(path) {
            Some(storepath) => loader.load(&storepath)?,
            None => return Err(Error::InvalidPath(path.clone())),
        }
    };
    let mut outdesc = desc.unwrap_or("").to_string();
    let mut outtags = tags.iter().map(|t| t.to_string()).collect::<Vec<_>>();
    if let Some(parent) = path.parent() {
        outtags.extend(implicit_tags_os_str(parent.file_name()));
    }
    let filenamestr = path
        .file_name()
        .ok_or(Error::InvalidPath(path.clone()))?
        .to_str()
        .ok_or(Error::InvalidPath(path.clone()))?;
    for FileData {
        path: pattern,
        desc: fdesc,
        tags: ftags,
    } in files
    {
        if glob_match(&pattern, filenamestr) {
            outtags.extend(
                ftags
                    .iter()
                    .map(|t| t.to_string())
                    .chain(implicit_tags_str(filenamestr)),
            );
            if let Some(fdesc) = fdesc {
                outdesc = format!("{}\n{}", fdesc, outdesc);
            }
        }
    }
    // Remove duplicate tags.
    outtags.sort();
    outtags.dedup();
    return Ok(full_description(outtags, outdesc));
}

fn what_is_dir(path: &PathBuf) -> Result<String, Error> {
    let mut loader = Loader::new(LoaderOptions::new(true, true, FileLoadingOptions::Skip));
    let DirData {
        desc,
        tags,
        files: _,
    } = {
        match get_store_path::<true>(path) {
            Some(storepath) => loader.load(&storepath)?,
            None => return Err(Error::InvalidPath(path.clone())),
        }
    };
    let desc = desc.unwrap_or("").to_string();
    let tags = tags
        .iter()
        .map(|t| t.to_string())
        .chain(implicit_tags_os_str(path.file_name()))
        .collect::<Vec<_>>();
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

pub(crate) fn untracked_files(root: PathBuf) -> Result<Vec<PathBuf>, Error> {
    use glob_match::glob_match;
    let mut walker = WalkDirectories::from(root.clone())?;
    let mut untracked: Vec<PathBuf> = Vec::new();
    let mut loader = Loader::new(LoaderOptions::new(
        false,
        false,
        FileLoadingOptions::Load {
            file_tags: false,
            file_desc: false,
        },
    ));
    while let Some((_depth, dirpath, children)) = walker.next() {
        let DirData {
            files: patterns,
            desc: _,
            tags: _,
        }: DirData = {
            match get_store_path::<true>(&dirpath) {
                Some(path) => loader.load(&path)?,
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
        untracked.extend(children.iter().filter_map(|child| {
            let fnamestr = child.name().to_str()?;
            if patterns.iter().any(|p| glob_match(&p.path, fnamestr)) {
                None
            } else {
                get_relative_path(&dirpath, &child.name(), &root)
            }
        }));
    }
    return Ok(untracked);
}

pub(crate) fn get_all_tags(path: PathBuf) -> Result<Vec<String>, Error> {
    let mut alltags: Vec<String> = Vec::new();
    let mut walker = WalkDirectories::from(path)?;
    let mut loader = Loader::new(LoaderOptions::new(
        true,
        false,
        FileLoadingOptions::Load {
            file_tags: true,
            file_desc: false,
        },
    ));
    while let Some((_depth, dirpath, _filenames)) = walker.next() {
        let DirData {
            mut tags,
            mut files,
            desc: _,
        } = {
            match get_store_path::<true>(&dirpath) {
                Some(path) => loader.load(&path)?,
                None => continue,
            }
        };
        alltags.extend(
            tags.drain(..)
                .map(|t| t.to_string())
                .chain(implicit_tags_os_str(dirpath.file_name())),
        );
        for mut fdata in files.drain(..) {
            alltags.extend(
                fdata
                    .tags
                    .drain(..)
                    .map(|t| t.to_string())
                    .chain(implicit_tags_str(fdata.path)),
            );
        }
    }
    alltags.sort();
    alltags.dedup();
    return Ok(alltags);
}
