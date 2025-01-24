use crate::{
    core::{Error, FTAG_BACKUP_FILE, FTAG_FILE},
    walk::DirEntry,
};
use aho_corasick::{AhoCorasick, Match};
use fast_glob::glob_match;
use std::{
    ffi::OsStr,
    fs::File,
    io::Read,
    ops::Range,
    path::{Path, PathBuf},
    sync::LazyLock,
};

/// Try to infer a range of years from the name of a document or file.
fn infer_year_range_str(mut input: &str) -> Option<Range<u16>> {
    if input.len() < 4 {
        return None;
    }
    let first: u16 = {
        let word = &input[..4];
        if word.chars().all(|b| b.is_ascii_digit()) {
            word.parse().ok()?
        } else {
            return None;
        }
    };
    input = &input[4..];
    if let Some(input) = input.strip_prefix('_') {
        if input.len() < 4 {
            return Some(first..(first + 1));
        }
        let word = &input[..4];
        if word.chars().all(|b| b.is_ascii_digit()) {
            let second = word.parse().unwrap_or(first);
            return Some(first..(second + 1));
        } else if let Some(input) = input.strip_prefix("to_") {
            if input.len() < 4 {
                return Some(first..(first + 1));
            }
            let word = &input[..4];
            if word.chars().all(|b| b.is_ascii_digit()) {
                let second = word.parse().unwrap_or(first);
                return Some(first..(second + 1));
            }
        }
    }
    Some(first..(first + 1))
}

/// Get an iterator over tags inferred from the format of the file. The input is
/// expected to be the path / name of the file.
fn infer_format_tag(input: &str) -> impl Iterator<Item = String> + use<'_> {
    const EXT_TAG_MAP: &[(&[&str], &str)] = &[
        (&[".mov", ".flv", ".mp4", ".3gp"], "video"),
        (&[".png", ".jpg", ".jpeg", ".bmp", ".webp", ".gif"], "image"),
    ];
    EXT_TAG_MAP.iter().filter_map(|(exts, tag)| {
        if exts
            .iter()
            .any(|ext| input[input.len().saturating_sub(ext.len())..].eq_ignore_ascii_case(ext))
        {
            Some(tag.to_string())
        } else {
            None
        }
    })
}

/// Get an iterator over all the implicit tags that can be inferred
/// from the name of the file or directory.
pub(crate) fn implicit_tags_str(name: &str) -> impl Iterator<Item = String> + '_ {
    infer_year_range_str(name)
        .unwrap_or(0..0)
        .map(|y| y.to_string())
        .chain(infer_format_tag(name))
}

/// Get the filename from the path as a string. If the path cannot be a valid
/// string, an error is returned. If the path doesn't exist, an empty string is
/// returned.
pub(crate) fn get_filename_str(path: &Path) -> Result<&str, Error> {
    Ok(match path.file_name() {
        Some(fname) => fname
            .to_str()
            .ok_or(Error::InvalidPath(path.to_path_buf()))?,
        None => "",
    })
}

/// This datastructure is responsible for finding matches between the
/// files on disk, and globs listed in the ftag file. This can be
/// reused for multiple folders to avoid reallocations.
pub(crate) struct GlobMatches {
    table: Vec<bool>, //The major index represents the files matched by a single glob.
    num_files: usize,
    num_globs: usize,
}

impl GlobMatches {
    pub fn new() -> GlobMatches {
        GlobMatches {
            table: Vec::new(),
            num_files: 0,
            num_globs: 0,
        }
    }

    /// Get a row from the table as a slice. The row indicates all the files
    /// that a particular glob matches.
    fn row(&self, gi: usize) -> &[bool] {
        let start = gi * self.num_files;
        &self.table[start..(start + self.num_files)]
    }

    /// Get the row and perfect match as mutable reference for the given glob
    /// index.
    fn row_mut(&mut self, gi: usize) -> &mut [bool] {
        let start = gi * self.num_files;
        &mut self.table[start..(start + self.num_files)]
    }

    /// Populate this struct with matches from a new set of `files` and
    /// `globs`. If `short_circuit_globs` is true, then each glob will be
    /// matched with at most 1 file on disk. This is useful when you're not
    /// interested in matching all possible files, but only interested in
    /// knowing if a glob matches at least one file.
    pub fn find_matches(
        &mut self,
        files: &[DirEntry],
        globs: &[GlobData],
        short_circuit_globs: bool,
    ) {
        self.num_files = files.len();
        self.num_globs = globs.len();
        self.table.clear();
        self.table.resize(files.len() * globs.len(), false);
        'globs: for (gi, g) in globs.iter().enumerate() {
            let row = self.row_mut(gi);
            /* A glob can either directly be a filename or a glob that matches
             * one or more files. Checking for glob matches is MUCH more
             * expensive than direct comparison. So for this glob, first we look
             * for a direct match with a filename. If we find a match, we don't
             * check the remaining files, and move on to the next glob. If and
             * ONLY IF we don't find a diret match with any of the files, we try
             * to match it as a glob. I have tested with and without this
             * optimization, and it makes a significant difference.
             */
            let gpath = OsStr::new(g.path);
            if let Ok(fi) = files.binary_search_by(move |f| f.name().cmp(gpath)) {
                row[fi] = true;
                continue 'globs;
            }
            for (fi, f) in files.iter().enumerate() {
                if glob_match(g.path.as_bytes(), f.name().as_encoded_bytes()) {
                    row[fi] = true;
                    if short_circuit_globs {
                        break;
                    }
                }
            }
        }
    }

    /// For a given file at `file_index`, get indices of all globs
    /// that matched the file.
    pub fn matched_globs(&self, file_index: usize) -> impl Iterator<Item = usize> + '_ {
        (0..self.num_globs).filter(move |gi| self.row(*gi)[file_index])
    }

    /// Check if the glob at `glob_index` matched at least one file.
    pub fn is_glob_matched(&self, glob_index: usize) -> bool {
        self.row(glob_index).iter().any(|m| *m)
    }
}

/// Get the path of the store file corresponding to `path`. `path` can
/// be a filepath, in which case the store file will be it's sibling,
/// or a directory path, in which case the store file will be it's
/// child.
pub fn get_ftag_path<const MUST_EXIST: bool>(path: &Path) -> Option<PathBuf> {
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
    out.push(FTAG_FILE);
    if MUST_EXIST && !out.exists() {
        None
    } else {
        Some(out)
    }
}

/// Get the path of the backup ftag file corresponding to `path`.
pub fn get_ftag_backup_path(path: &Path) -> PathBuf {
    let mut dirpath = if path.is_dir() {
        PathBuf::from(path)
    } else {
        let mut out = PathBuf::from(path);
        out.pop();
        out
    };
    dirpath.push(FTAG_BACKUP_FILE);
    dirpath
}

/// Loads and parses an ftag file. Reuse this to avoid allocations.
pub(crate) struct Loader {
    raw_text: String,
    options: LoaderOptions,
}

/// Data in an ftag file, corresponding to one file / glob.
pub(crate) struct GlobData<'a> {
    pub desc: Option<&'a str>,
    pub path: &'a str,
    pub tags: Vec<&'a str>,
}

/// Data from an ftag file.
pub(crate) struct DirData<'a> {
    pub desc: Option<&'a str>,
    pub tags: Vec<&'a str>,
    pub globs: Vec<GlobData<'a>>,
}

/// Options for loading the file data from an ftag file.
pub(crate) enum FileLoadingOptions {
    /// Skip loading the file data altogether.
    Skip,
    /// `file_tags` controls whether the tags of files are
    /// loaded. `file_desc` controls whether the descriptions of files
    /// are loaded.
    Load { file_tags: bool, file_desc: bool },
}

/// Options for loading data from an ftag file.
pub(crate) struct LoaderOptions {
    /// Load tags of the directory.
    dir_tags: bool,
    /// Load description of the directory.
    dir_desc: bool,
    /// Options for loading file data.
    file_options: FileLoadingOptions,
}

impl LoaderOptions {
    pub fn new(dir_tags: bool, dir_desc: bool, file_options: FileLoadingOptions) -> Self {
        LoaderOptions {
            dir_tags,
            dir_desc,
            file_options,
        }
    }
}

static AC_PARSER: LazyLock<AhoCorasick> = LazyLock::new(|| {
    const HEADER_STR: [&str; 3] = ["[path]", "[tags]", "[desc]"];
    AhoCorasick::new(HEADER_STR).expect("FATAL: Unable to initialize the parser")
});

enum HeaderType {
    Path,
    Tags,
    Desc,
}

impl HeaderType {
    pub fn from_u32(i: u32) -> Option<Self> {
        match i {
            0 => Some(Self::Path),
            1 => Some(Self::Tags),
            2 => Some(Self::Desc),
            _ => None,
        }
    }
}

struct Header {
    kind: HeaderType,
    start: usize,
    end: usize,
}

impl Header {
    pub fn from_match(mat: Match) -> Option<Self> {
        HeaderType::from_u32(mat.pattern().as_u32()).map(|kind| Header {
            kind,
            start: mat.start(),
            end: mat.end(),
        })
    }
}

impl Loader {
    pub fn new(options: LoaderOptions) -> Loader {
        Loader {
            raw_text: String::new(),
            options,
        }
    }

    /// Check whether the file description should be loaded.
    pub fn include_file_desc(&self) -> bool {
        match self.options.file_options {
            FileLoadingOptions::Skip => false,
            FileLoadingOptions::Load {
                file_tags: _,
                file_desc,
            } => file_desc,
        }
    }

    /// Check whether the file tags should be loaded.
    pub fn include_file_tags(&self) -> bool {
        match self.options.file_options {
            FileLoadingOptions::Skip => false,
            FileLoadingOptions::Load {
                file_tags,
                file_desc: _,
            } => file_tags,
        }
    }

    /// Load the data from a .ftag file specified by the filepath.
    pub fn load<'a>(&'a mut self, filepath: &Path) -> Result<DirData<'a>, Error> {
        self.raw_text.clear();
        File::open(filepath)
            .map_err(|_| Error::CannotReadStoreFile(filepath.to_path_buf()))?
            .read_to_string(&mut self.raw_text)
            .map_err(|_| Error::CannotReadStoreFile(filepath.to_path_buf()))?;
        let input = self.raw_text.trim();
        // TODO: Consider checking if the file begins with a header.
        let mut headers = AC_PARSER.find_iter(input);
        let mut tags: Vec<&str> = Vec::new();
        let mut desc: Option<&str> = None;
        let mut files: Vec<GlobData<'a>> = Vec::new();
        // We store the data of the file we're currently parsing as:
        // (list of globs, list of tags, optional description).
        let mut curfile: Option<(Vec<&'a str>, Vec<&'a str>, Option<&'a str>)> = None;
        // Begin parsing.
        let (mut header, mut content, mut next_header) = match headers.next() {
            Some(mat) => {
                let h = Header::from_match(mat).ok_or(Error::CannotParseFtagFile(
                    filepath.to_path_buf(),
                    "FATAL: Error when searching for headers in the file.".into(),
                ))?;
                let (c, n) = match headers.next() {
                    Some(mat) => {
                        let n = Header::from_match(mat).ok_or(Error::CannotParseFtagFile(
                            filepath.to_path_buf(),
                            "FATAL: Error when searching for headers in the file.".into(),
                        ))?;
                        let c = input[h.end..n.start].trim();
                        (c, Some(n))
                    }
                    None => (input[mat.end()..].trim(), None),
                };
                (h, c, n)
            }
            None => {
                return Err(Error::CannotParseFtagFile(
                    filepath.to_path_buf(),
                    "File does not contain any headers.".into(),
                ))
            }
        };
        // Parse until no more headers are found.
        loop {
            match header.kind {
                HeaderType::Path => {
                    if let FileLoadingOptions::Skip = self.options.file_options {
                        break; // Stop parsing the file.
                    }
                    let newfile = (content.lines().collect(), Vec::new(), None);
                    if let Some((globs, tags, desc)) =
                        std::mem::replace(&mut curfile, Some(newfile))
                    {
                        for g in globs {
                            files.push(GlobData {
                                desc,
                                path: g,
                                tags: tags.clone(),
                            })
                        }
                    }
                }
                HeaderType::Tags => {
                    if let Some(file) = &mut curfile {
                        if self.include_file_tags() {
                            let (globs, tags, _desc) = file;
                            if tags.is_empty() {
                                tags.extend(content.split_whitespace().map(|w| w.trim()));
                            } else {
                                return Err(Error::CannotParseFtagFile(
                                    filepath.to_path_buf(),
                                    format!(
                                    "The following globs have more than one 'tags' header:\n{}.",
                                    globs.join("\n")
                                ),
                                ));
                            }
                        }
                    } else if self.options.dir_tags {
                        if tags.is_empty() {
                            tags.extend(content.split_whitespace().map(|w| w.trim()));
                        } else {
                            return Err(Error::CannotParseFtagFile(
                                filepath.to_path_buf(),
                                "The directory has more than one 'tags' header.".into(),
                            ));
                        }
                    }
                }
                HeaderType::Desc => {
                    if let Some(file) = &mut curfile {
                        if self.include_file_desc() {
                            let (globs, _tags, desc) = file;
                            if desc.is_some() {
                                return Err(Error::CannotParseFtagFile(
                                    filepath.to_path_buf(),
                                    format!(
                                        "Following globs have more than one description:\n{}.",
                                        globs.join("\n")
                                    ),
                                ));
                            } else {
                                *desc = Some(content);
                            }
                        }
                    } else if self.options.dir_desc {
                        if desc.is_some() {
                            return Err(Error::CannotParseFtagFile(
                                filepath.to_path_buf(),
                                "The directory has more than one description.".into(),
                            ));
                        } else {
                            desc = Some(content);
                        }
                    }
                }
            };
            match next_header {
                Some(next) => {
                    header = next;
                    (content, next_header) = match headers.next() {
                        Some(mat) => {
                            let n = Header::from_match(mat).ok_or(Error::CannotParseFtagFile(
                                filepath.to_path_buf(),
                                "FATAL: Error when searching for headers in the file.".into(),
                            ))?;
                            content = input[header.end..n.start].trim();
                            (content, Some(n))
                        }
                        None => (input[header.end..].trim(), None),
                    };
                }
                None => break,
            }
        }
        if let Some((globs, tags, desc)) = curfile {
            for g in globs {
                files.push(GlobData {
                    desc,
                    path: g,
                    tags: tags.clone(),
                })
            }
        }
        Ok(DirData {
            desc,
            tags,
            globs: files,
        })
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn t_infer_year_range() {
        let inputs = vec!["2021_to_2023", "2021_2023"];
        let expected = vec!["2021", "2022", "2023"];
        for input in inputs {
            let actual: Vec<_> = implicit_tags_str(input).collect();
            assert_eq!(actual, expected);
        }
        let inputs = vec!["1998_MyDirectory", "1998_MyFile.pdf"];
        let expected = vec!["1998"];
        for input in inputs {
            let actual: Vec<_> = implicit_tags_str(input).collect();
            assert_eq!(actual, expected);
        }
    }

    #[test]
    fn t_infer_format_tags() {
        let inputs = &["test.gif", "ex", "test2.png", "myvid.mov"];
        let expected: &[&[&str]] = &[&["image"], &[], &["image"], &["video"]];
        for (input, expected) in inputs.iter().zip(expected.iter()) {
            let actual: Vec<_> = infer_format_tag(input).collect();
            assert_eq!(&actual, expected);
        }
    }
}
