use crate::{
    core::{Error, FTAG_BACKUP_FILE, FTAG_FILE},
    walk::DirEntry,
};
use aho_corasick::{AhoCorasick, Match};
use fast_glob::glob_match;
use smallvec::SmallVec;
use std::{
    ffi::OsStr,
    fmt::Display,
    fs::File,
    io::Read,
    ops::Range,
    path::{Path, PathBuf},
    sync::LazyLock,
};

pub(crate) enum Tag<'a> {
    Text(&'a str),
    Year(u16),
    Format(&'a str),
}

impl Display for Tag<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Tag::Text(t) | Tag::Format(t) => write!(f, "{}", t),
            Tag::Year(y) => write!(f, "{}", y),
        }
    }
}

/// Try to infer a range of years from the name of a document or file.
fn infer_year_range(mut input: &str) -> Option<Range<u16>> {
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
fn infer_format_tag(input: &str) -> impl Iterator<Item = Tag> + use<'_> {
    const EXT_TAG_MAP: &[(&[&str], &str)] = &[
        (&[".mov", ".flv", ".mp4", ".3gp"], "video"),
        (&[".png", ".jpg", ".jpeg", ".bmp", ".webp", ".gif"], "image"),
    ];
    EXT_TAG_MAP.iter().filter_map(|(exts, tag)| {
        if exts
            .iter()
            .any(|ext| input[input.len().saturating_sub(ext.len())..].eq_ignore_ascii_case(ext))
        {
            Some(Tag::Format(tag))
        } else {
            None
        }
    })
}

/// Get an iterator over all the implicit tags that can be inferred
/// from the name of the file or directory.
pub(crate) fn infer_implicit_tags(name: &str) -> impl Iterator<Item = Tag> + use<'_> {
    infer_year_range(name)
        .into_iter()
        .flatten()
        .map(Tag::Year)
        .chain(infer_format_tag(name))
}

/// Get the filename from the path as a string. If the path cannot be a valid
/// string, an error is returned. If the path doesn't exist, an empty string is
/// returned.
pub(crate) fn get_filename_str(path: &Path) -> Result<&str, Error> {
    match path.file_name() {
        Some(fname) => match fname.to_str() {
            Some(fname) => Ok(fname),
            None => return Err(Error::InvalidPath(path.to_path_buf())),
        },
        None => Ok(""),
    }
}

/// This datastructure is responsible for finding matches between the
/// files on disk, and globs listed in the ftag file. This can be
/// reused for multiple folders to avoid reallocations.
pub(crate) struct GlobMatches {
    file_matches: Vec<SmallVec<[usize; 4]>>,
    glob_matches: Vec<bool>,
}

impl GlobMatches {
    pub fn new() -> GlobMatches {
        GlobMatches {
            file_matches: Vec::new(),
            glob_matches: Vec::new(),
        }
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
        self.file_matches.clear();
        self.file_matches.resize(files.len(), SmallVec::new());
        self.glob_matches.clear();
        self.glob_matches.resize(globs.len(), false);
        'globs: for (gi, g) in globs.iter().enumerate() {
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
                self.file_matches[fi].push(gi);
                self.glob_matches[gi] = true;
                continue 'globs;
            }
            for (fi, f) in files.iter().enumerate() {
                if glob_match(g.path.as_bytes(), f.name().as_encoded_bytes()) {
                    self.file_matches[fi].push(gi);
                    self.glob_matches[gi] = true;
                    if short_circuit_globs {
                        break;
                    }
                }
            }
        }
    }

    /// For a given file at `file_index`, get indices of all globs
    /// that matched the file.
    pub fn matched_globs(&self, file_index: usize) -> impl Iterator<Item = usize> + use<'_> {
        self.file_matches[file_index].iter().copied()
    }

    /// Check if the glob at `glob_index` matched at least one file.
    pub fn is_glob_matched(&self, glob_index: usize) -> bool {
        self.glob_matches[glob_index]
    }

    pub fn is_file_matched(&self, file_index: usize) -> bool {
        !self.file_matches[file_index].is_empty()
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
#[derive(Clone)]
pub(crate) struct GlobData<'a> {
    pub desc: Option<&'a str>,
    pub path: &'a str,
    tags: Range<usize>,
}

/// Data from an ftag file.
pub(crate) struct DirData<'a> {
    pub alltags: Vec<&'a str>,
    pub desc: Option<&'a str>,
    tags: Range<usize>,
    pub globs: Vec<GlobData<'a>>,
}

impl Default for DirData<'_> {
    fn default() -> Self {
        Self {
            alltags: Vec::with_capacity(32),
            desc: None,
            tags: 0..0,
            globs: Vec::with_capacity(32),
        }
    }
}

impl<'a> GlobData<'a> {
    pub fn tags(&'a self, alltags: &'a [&'a str]) -> &'a [&'a str] {
        &alltags[self.tags.start..self.tags.end]
    }
}

impl<'a> DirData<'a> {
    pub fn tags(&'a self) -> &'a [&'a str] {
        &self.alltags[self.tags.start..self.tags.end]
    }
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

    /// Check whether the file description should be loaded.
    pub fn include_file_desc(&self) -> bool {
        match self.file_options {
            FileLoadingOptions::Skip => false,
            FileLoadingOptions::Load {
                file_tags: _,
                file_desc,
            } => file_desc,
        }
    }

    /// Check whether the file tags should be loaded.
    pub fn include_file_tags(&self) -> bool {
        match self.file_options {
            FileLoadingOptions::Skip => false,
            FileLoadingOptions::Load {
                file_tags,
                file_desc: _,
            } => file_tags,
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

fn load_impl<'text>(
    input: &'text str,
    filepath: &Path,
    options: &LoaderOptions,
    dst: &mut DirData<'text>,
) -> Result<(), Error> {
    let DirData {
        alltags,
        desc,
        tags: dirtags,
        globs: files,
    } = dst;
    let mut headers = AC_PARSER.find_iter(input);
    // We store the data of the file we're currently parsing as:
    // (list of globs, list of tags, optional description).
    let mut current_unit: Option<(&str, Range<usize>, Option<&str>)> = None;
    // Begin parsing.
    let (mut header, mut content, mut next_header) = match headers.next() {
        Some(mat) => {
            let h = match Header::from_match(mat) {
                Some(h) => h,
                None => {
                    return Err(Error::CannotParseFtagFile(
                        filepath.to_path_buf(),
                        "FATAL: Error when searching for headers in the file.".into(),
                    ))
                }
            };
            let (c, n) = match headers.next() {
                Some(mat) => {
                    let n = match Header::from_match(mat) {
                        Some(n) => n,
                        None => {
                            return Err(Error::CannotParseFtagFile(
                                filepath.to_path_buf(),
                                "FATAL: Error when searching for headers in the file.".into(),
                            ))
                        }
                    };
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
                if let FileLoadingOptions::Skip = options.file_options {
                    break; // Stop parsing the file.
                }
                match current_unit.as_mut() {
                    Some((globs, tags, desc)) => {
                        let desc = desc.take();
                        let tags = std::mem::replace(tags, 0..0);
                        let lines = std::mem::replace(globs, content).lines();
                        files.extend(lines.map(|g| GlobData {
                            desc,
                            path: g.trim(),
                            tags: tags.clone(),
                        }));
                    }
                    None => current_unit = Some((content, 0..0, None)),
                }
            }
            HeaderType::Tags => {
                if let Some((globs, tags, _desc)) = current_unit.as_mut() {
                    if options.include_file_tags() {
                        if tags.start == tags.end {
                            // No tags found for the current unit.
                            let before = alltags.len();
                            alltags.extend(content.split_whitespace());
                            *tags = before..alltags.len();
                        } else {
                            return Err(Error::CannotParseFtagFile(
                                filepath.to_path_buf(),
                                format!(
                                    "The following globs have more than one 'tags' header:\n{}.",
                                    globs
                                ),
                            ));
                        }
                    }
                } else if options.dir_tags {
                    if dirtags.start == dirtags.end {
                        // No directory tags found.
                        let before = alltags.len();
                        alltags.extend(content.split_whitespace());
                        *dirtags = before..alltags.len();
                    } else {
                        return Err(Error::CannotParseFtagFile(
                            filepath.to_path_buf(),
                            "The directory has more than one 'tags' header.".into(),
                        ));
                    }
                }
            }
            HeaderType::Desc => {
                if let Some(file) = &mut current_unit {
                    if options.include_file_desc() {
                        let (globs, _tags, desc) = file;
                        if desc.is_some() {
                            return Err(Error::CannotParseFtagFile(
                                filepath.to_path_buf(),
                                format!(
                                    "Following globs have more than one description:\n{}.",
                                    globs
                                ),
                            ));
                        } else {
                            *desc = Some(content);
                        }
                    }
                } else if options.dir_desc {
                    if desc.is_some() {
                        return Err(Error::CannotParseFtagFile(
                            filepath.to_path_buf(),
                            "The directory has more than one description.".into(),
                        ));
                    } else {
                        *desc = Some(content);
                    }
                }
            }
        };
        match next_header {
            Some(next) => {
                header = next;
                (content, next_header) = match headers.next() {
                    Some(mat) => {
                        let n = match Header::from_match(mat) {
                            Some(n) => n,
                            None => {
                                return Err(Error::CannotParseFtagFile(
                                    filepath.to_path_buf(),
                                    "FATAL: Error when searching for headers in the file.".into(),
                                ))
                            }
                        };
                        content = input[header.end..n.start].trim();
                        (content, Some(n))
                    }
                    None => (input[header.end..].trim(), None),
                };
            }
            None => break,
        }
    }
    if let Some((globs, tags, desc)) = current_unit {
        files.extend(globs.lines().map(|g| GlobData {
            desc,
            path: g.trim(),
            tags: tags.clone(),
        }));
    }
    Ok(())
}

impl Loader {
    pub fn new(options: LoaderOptions) -> Loader {
        Loader {
            raw_text: String::new(),
            options,
        }
    }

    /// Load the data from a .ftag file specified by the filepath.
    pub fn load<'a>(&'a mut self, filepath: &Path) -> Result<DirData<'a>, Error> {
        self.raw_text.clear();
        File::open(filepath)
            .map_err(|_| Error::CannotReadStoreFile(filepath.to_path_buf()))?
            .read_to_string(&mut self.raw_text)
            .map_err(|_| Error::CannotReadStoreFile(filepath.to_path_buf()))?;
        let mut out = DirData::default();
        load_impl(self.raw_text.trim(), filepath, &self.options, &mut out)?;
        Ok(out)
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
            let actual: Vec<_> = infer_implicit_tags(input).map(|t| t.to_string()).collect();
            assert_eq!(actual, expected);
        }
        let inputs = vec!["1998_MyDirectory", "1998_MyFile.pdf"];
        let expected = vec!["1998"];
        for input in inputs {
            let actual: Vec<_> = infer_implicit_tags(input).map(|t| t.to_string()).collect();
            assert_eq!(actual, expected);
        }
    }

    #[test]
    fn t_infer_format_tags() {
        let inputs = &["test.gif", "ex", "test2.png", "myvid.mov"];
        let expected: &[&[&str]] = &[&["image"], &[], &["image"], &["video"]];
        for (input, expected) in inputs.iter().zip(expected.iter()) {
            let actual: Vec<_> = infer_format_tag(input).map(|t| t.to_string()).collect();
            assert_eq!(&actual, expected);
        }
    }
}
