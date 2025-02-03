use crate::{
    filter::FilterParseError,
    load::{
        get_filename_str, get_ftag_backup_path, get_ftag_path, infer_implicit_tags, DirData,
        FileLoadingOptions, GlobMatches, Loader, LoaderOptions,
    },
    walk::{DirTree, MetaData, VisitedDir},
};
use std::{
    collections::HashSet,
    fmt::Debug,
    fs::OpenOptions,
    io,
    path::{Path, PathBuf},
};

pub(crate) const FTAG_FILE: &str = ".ftag";
pub(crate) const FTAG_BACKUP_FILE: &str = ".ftagbak";

/// The data related to a glob in an ftag file. This is meant to be used in
/// error reporting.
pub struct GlobInfo {
    glob: String,
    dirpath: PathBuf, // The store file where the glob was found.
}

pub enum Error {
    TUIFailure(String),
    GUIFailure(eframe::Error),
    EditCommandFailed(String),
    UnmatchedGlobs(Vec<GlobInfo>),
    InvalidArgs,
    InvalidWorkingDirectory,
    InvalidPath(PathBuf),
    CannotReadStoreFile(PathBuf),
    CannotParseFtagFile(PathBuf, String),
    CannotWriteFile(PathBuf),
    InvalidFilter(FilterParseError),
    DirectoryTraversalFailed,
}

impl Debug for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TUIFailure(message) => {
                write!(f, "Something went wrong in interactive mode:\n{}", message)
            }
            Self::GUIFailure(e) => write!(f, "Failure in the GUI:\n{}", e),
            Self::EditCommandFailed(message) => write!(f, "Unable to edit file:\n{}", message),
            Self::UnmatchedGlobs(infos) => {
                writeln!(f)?;
                for info in infos {
                    writeln!(
                        f,
                        "No files in '{}' matching '{}'",
                        info.dirpath.display(),
                        info.glob
                    )?;
                }
                Ok(())
            }
            Self::InvalidArgs => write!(f, "Invalid command line arguments"),
            Self::InvalidWorkingDirectory => write!(f, "This is not a valid working directory."),
            Self::InvalidPath(path) => write!(f, "'{}' is not a valid path.", path.display()),
            Self::CannotReadStoreFile(path) => {
                write!(f, "Unable to read file: '{}'", path.display())
            }
            Self::CannotParseFtagFile(path, message) => {
                writeln!(f, "While parsing file '{}'", path.display())?;
                write!(f, "{}", message)
            }
            Self::CannotWriteFile(path) => writeln!(f, "Cannot write to file {}", path.display()),
            Self::InvalidFilter(err) => write!(f, "Unable to parse filter:\n{:?}", err),
            Self::DirectoryTraversalFailed => {
                write!(f, "Something went wrong when traversing directories.")
            }
        }
    }
}

/// Recursively check all directories. This will read all .ftag
/// files, and make sure every listed glob / path matches at least one
/// file on disk.
pub fn check(path: PathBuf) -> Result<(), Error> {
    let mut matcher = GlobMatches::new();
    let mut missing = Vec::new();
    let mut dir = DirTree::new(
        path.clone(),
        LoaderOptions::new(
            false,
            false,
            FileLoadingOptions::Load {
                file_tags: false,
                file_desc: false,
            },
        ),
    )?;
    while let Some(VisitedDir {
        rel_dir_path,
        files,
        metadata,
        ..
    }) = dir.walk()
    {
        match metadata {
            MetaData::FailedToLoad(e) => return Err(e),
            MetaData::NotFound => continue, // No metadata.
            MetaData::Ok(DirData { globs, .. }) => {
                matcher.find_matches(files, globs, true);
                missing.extend(globs.iter().enumerate().filter_map(|(i, f)| {
                    if !matcher.is_glob_matched(i) {
                        Some(GlobInfo {
                            glob: f.path.to_string(),
                            dirpath: rel_dir_path.to_path_buf(),
                        })
                    } else {
                        None
                    }
                }));
            }
        }
    }
    if missing.is_empty() {
        Ok(())
    } else {
        Err(Error::UnmatchedGlobs(missing))
    }
}

struct FileDataOwned {
    glob: String,
    tags: Vec<String>,
    desc: Option<String>,
}

struct FileDataMultiple {
    globs: Vec<String>,
    tags: Vec<String>,
    desc: Option<String>,
}

fn write_globs<T: AsRef<str>>(globs: &[T], w: &mut impl io::Write) -> Result<(), io::Error> {
    if globs.is_empty() {
        return Ok(());
    }
    writeln!(w, "\n[path]")?;
    for glob in globs.iter().map(|g| g.as_ref()) {
        writeln!(w, "{}", glob)?;
    }
    Ok(())
}

fn write_tags<T: AsRef<str>>(tags: &[T], w: &mut impl io::Write) -> Result<(), io::Error> {
    if tags.is_empty() {
        return Ok(());
    }
    writeln!(w, "[tags]")?;
    if tags
        .iter()
        .try_fold(0usize, |len, tag| -> Result<usize, io::Error> {
            let tag = tag.as_ref();
            Ok(if len > 80 {
                writeln!(w, "{}", tag)?;
                0usize
            } else {
                write!(w, "{} ", tag)?;
                len + tag.len() + 1
            })
        })?
        > 0
    {
        writeln!(w)?;
    }
    Ok(())
}

fn write_desc<T: AsRef<str>>(desc: Option<&T>, w: &mut impl io::Write) -> Result<(), io::Error> {
    match desc {
        Some(desc) => writeln!(w, "[desc]\n{}", desc.as_ref()),
        None => Ok(()),
    }
}

pub fn clean(path: PathBuf) -> Result<(), Error> {
    let mut matcher = GlobMatches::new();
    let mut valid: Vec<FileDataOwned> = Vec::new();
    let mut dir = DirTree::new(
        path,
        LoaderOptions::new(
            true,
            true,
            FileLoadingOptions::Load {
                file_tags: true,
                file_desc: true,
            },
        ),
    )?;
    while let Some(VisitedDir {
        abs_dir_path,
        files,
        metadata,
        ..
    }) = dir.walk()
    {
        let data = match metadata {
            MetaData::Ok(d) => d,
            MetaData::NotFound => continue,
            MetaData::FailedToLoad(e) => return Err(e),
        };
        matcher.find_matches(files, &data.globs, true);
        valid.clear();
        valid.extend(data.globs.iter().enumerate().filter_map(|(gi, g)| {
            if matcher.is_glob_matched(gi) {
                let mut tags: Vec<String> = g
                    .tags(&data.alltags)
                    .iter()
                    .map(|t| t.to_string())
                    .collect();
                tags.sort_unstable();
                tags.dedup();
                Some(FileDataOwned {
                    glob: g.path.to_string(),
                    tags,
                    desc: g.desc.map(|d| d.to_string()),
                })
            } else {
                None
            }
        }));
        // This should group files that share the same tags and desc
        valid.sort_unstable_by(|a, b| match a.tags.cmp(&b.tags) {
            std::cmp::Ordering::Less => std::cmp::Ordering::Less,
            std::cmp::Ordering::Equal => a.desc.cmp(&b.desc),
            std::cmp::Ordering::Greater => std::cmp::Ordering::Greater,
        });

        let fpath = get_ftag_path::<true>(abs_dir_path)
            .ok_or(Error::CannotReadStoreFile(abs_dir_path.to_path_buf()))?;
        // Backup existing data.
        std::fs::copy(&fpath, get_ftag_backup_path(abs_dir_path))
            .map_err(|_| Error::CannotWriteFile(fpath.clone()))?;
        let mut writer = io::BufWriter::new(
            OpenOptions::new()
                .write(true)
                .truncate(true)
                .create(true)
                .open(&fpath)
                .map_err(|_| Error::CannotWriteFile(fpath.clone()))?,
        );
        // Write directory data.
        write_tags(data.tags(), &mut writer).map_err(|_| Error::CannotWriteFile(fpath.clone()))?;
        write_desc(data.desc.as_ref(), &mut writer)
            .map_err(|_| Error::CannotWriteFile(fpath.clone()))?;
        // Write out the file data in groups that share the same tags and description.
        if let Some(last) = valid
            .drain(..)
            .try_fold(
                None,
                |current: Option<FileDataMultiple>,
                 file|
                 -> Result<Option<FileDataMultiple>, io::Error> {
                    Ok(match current {
                        Some(mut current)
                            if current.tags == file.tags && current.desc == file.desc =>
                        {
                            current.globs.push(file.glob);
                            Some(current)
                        }
                        Some(current) => {
                            write_globs(&current.globs, &mut writer)?;
                            write_tags(&current.tags, &mut writer)?;
                            write_desc(current.desc.as_ref(), &mut writer)?;
                            Some(FileDataMultiple {
                                globs: vec![file.glob],
                                tags: file.tags,
                                desc: file.desc,
                            })
                        }
                        None => Some(FileDataMultiple {
                            globs: vec![file.glob],
                            tags: file.tags,
                            desc: file.desc,
                        }),
                    })
                },
            )
            .map_err(|_| Error::CannotWriteFile(fpath.clone()))?
        {
            // This is the last entry.
            write_globs(&last.globs, &mut writer)
                .map_err(|_| Error::CannotWriteFile(fpath.clone()))?;
            write_tags(&last.tags, &mut writer)
                .map_err(|_| Error::CannotWriteFile(fpath.clone()))?;
            write_desc(last.desc.as_ref(), &mut writer)
                .map_err(|_| Error::CannotWriteFile(fpath.clone()))?;
        }
    }
    Ok(())
}

/// Get a description string from the tags and description of a file.
fn full_description(tags: Vec<String>, desc: String) -> String {
    let tagstr = {
        let mut tags = tags.into_iter();
        let first = tags.next().unwrap_or_default();
        tags.fold(first, |acc, t| format!("{}, {}", acc, t))
    };
    format!(
        "tags: [{}]{}",
        tagstr,
        if desc.is_empty() {
            desc
        } else {
            format!("\n{}", desc)
        }
    )
}

/// Get the description of a file or a directory.
pub fn what_is(path: &Path) -> Result<String, Error> {
    if path.is_file() {
        what_is_file(path)
    } else if path.is_dir() {
        what_is_dir(path)
    } else {
        Err(Error::InvalidPath(path.to_path_buf()))
    }
}

/// Get a full description of the file that includes the tags and the
/// description of said file.
fn what_is_file(path: &Path) -> Result<String, Error> {
    use fast_glob::glob_match;
    let mut loader = Loader::new(LoaderOptions::new(
        true,
        true,
        FileLoadingOptions::Load {
            file_tags: true,
            file_desc: true,
        },
    ));
    let data = match get_ftag_path::<true>(path) {
        Some(storepath) => loader.load(&storepath)?,
        None => return Err(Error::InvalidPath(path.to_path_buf())),
    };
    let mut outdesc = data.desc.unwrap_or("").to_string();
    let mut outtags = data
        .tags()
        .iter()
        .map(|t| t.to_string())
        .collect::<Vec<_>>();
    if let Some(parent) = path.parent() {
        outtags.extend(infer_implicit_tags(get_filename_str(parent)?).map(|t| t.to_string()));
    }
    let filenamestr = path
        .file_name()
        .ok_or(Error::InvalidPath(path.to_path_buf()))?
        .to_str()
        .ok_or(Error::InvalidPath(path.to_path_buf()))?;
    for g in data.globs.iter() {
        if glob_match(g.path, filenamestr) {
            outtags.extend(
                g.tags(&data.alltags)
                    .iter()
                    .map(|t| t.to_string())
                    .chain(infer_implicit_tags(filenamestr).map(|t| t.to_string())),
            );
            if let Some(fdesc) = g.desc {
                outdesc = format!("{}\n{}", fdesc, outdesc);
            }
        }
    }
    // Remove duplicate tags.
    outtags.sort_unstable();
    outtags.dedup();
    Ok(full_description(outtags, outdesc))
}

/// Get the full description of a directory that includes it's tags and
/// description.
fn what_is_dir(path: &Path) -> Result<String, Error> {
    let mut loader = Loader::new(LoaderOptions::new(true, true, FileLoadingOptions::Skip));
    let data = match get_ftag_path::<true>(path) {
        Some(storepath) => loader.load(&storepath)?,
        None => return Err(Error::InvalidPath(path.to_path_buf())),
    };
    let desc = data.desc.unwrap_or("").to_string();
    let tags = data
        .tags()
        .iter()
        .map(|t| t.to_string())
        .chain(infer_implicit_tags(get_filename_str(path)?).map(|t| t.to_string()))
        .collect::<Vec<_>>();
    Ok(full_description(tags, desc))
}

/// Recursively traverse the directories starting from `root` and
/// return all files that are not tracked.
pub fn untracked_files(root: PathBuf) -> Result<Vec<PathBuf>, Error> {
    let mut matcher = GlobMatches::new();
    let mut dir = DirTree::new(
        root.clone(),
        LoaderOptions::new(
            false,
            false,
            FileLoadingOptions::Load {
                file_tags: false,
                file_desc: false,
            },
        ),
    )?;
    let mut untracked = Vec::new();
    while let Some(VisitedDir {
        rel_dir_path,
        files,
        metadata,
        ..
    }) = dir.walk()
    {
        match metadata {
            MetaData::FailedToLoad(e) => return Err(e),
            MetaData::Ok(DirData { globs, .. }) => {
                matcher.find_matches(files, globs, false);
                untracked.extend(files.iter().enumerate().filter_map(|(fi, file)| {
                    // Skip the files that matched with at least one glob. Copy the
                    // paths of files that didn't match with any glob.
                    match matcher.is_file_matched(fi) {
                        true => None,
                        false => {
                            let mut relpath = rel_dir_path.to_path_buf();
                            relpath.push(file.name());
                            Some(relpath)
                        }
                    }
                }));
            }
            MetaData::NotFound => {
                // Metadata doesn't exist so everything is untracked.
                untracked.extend(files.iter().map(|ch| {
                    let mut relpath = rel_dir_path.to_path_buf();
                    relpath.push(ch.name());
                    relpath
                }));
            }
        }
    }
    Ok(untracked)
}

/// Recursively traverse the directories from `path` and get all tags.
pub fn get_all_tags(path: PathBuf) -> Result<impl Iterator<Item = String>, Error> {
    let mut alltags = HashSet::new();
    let mut matcher = GlobMatches::new();
    let mut dir = DirTree::new(
        path,
        LoaderOptions::new(
            true,
            false,
            FileLoadingOptions::Load {
                file_tags: true,
                file_desc: false,
            },
        ),
    )?;
    while let Some(VisitedDir {
        rel_dir_path,
        metadata,
        files,
        ..
    }) = dir.walk()
    {
        match metadata {
            MetaData::FailedToLoad(e) => return Err(e), // Bail out with error.
            MetaData::Ok(DirData {
                alltags: tags,
                globs,
                ..
            }) => {
                alltags.extend(tags.iter().map(|t| t.to_string()).chain(
                    infer_implicit_tags(get_filename_str(rel_dir_path)?).map(|t| t.to_string()),
                ));
                matcher.find_matches(files, globs, false);
                alltags.extend(
                    files
                        .iter()
                        .enumerate()
                        .filter(|(fi, _f)| matcher.is_file_matched(*fi))
                        .filter_map(|(_fi, f)| f.name().to_str())
                        .flat_map(|t| infer_implicit_tags(t).map(|t| t.to_string())),
                );
            }
            MetaData::NotFound => continue, // No metadata, just pass on the tags to the next dir.
        }
    }
    Ok(alltags.into_iter())
}

fn match_desc(words: &[String], tags: &[&str], desc: Option<&str>) -> bool {
    tags.iter().any(|tag| {
        // Check if tag matches
        let lower = tag.to_lowercase();
        words
            .iter()
            .any(|word| lower.matches(word).next().is_some())
    }) || match desc {
        // Check if description matches.
        Some(desc) => {
            let desc = desc.to_lowercase();
            words.iter().any(|word| desc.matches(word).next().is_some())
        }
        None => false,
    }
}

pub fn search(path: PathBuf, needle: &str) -> Result<(), Error> {
    let words: Vec<_> = needle
        .trim()
        .split(|c: char| !c.is_alphanumeric())
        .map(|word| word.trim().to_lowercase())
        .collect();
    let mut dir = DirTree::new(
        path,
        LoaderOptions::new(
            true,
            true,
            FileLoadingOptions::Load {
                file_tags: true,
                file_desc: true,
            },
        ),
    )?;
    while let Some(VisitedDir { metadata, .. }) = dir.walk() {
        match metadata {
            MetaData::FailedToLoad(e) => return Err(e),
            MetaData::Ok(data) => {
                let dirmatch = match_desc(&words, data.tags(), data.desc);
                for filepath in data.globs.iter().filter_map(|g| {
                    if dirmatch || match_desc(&words, g.tags(&data.alltags), g.desc) {
                        Some(g.path)
                    } else {
                        None
                    }
                }) {
                    println!("{}", filepath);
                }
            }
            MetaData::NotFound => continue, // No metadata, just keep going.
        }
    }
    Ok(())
}
