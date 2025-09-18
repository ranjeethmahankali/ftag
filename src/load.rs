use crate::{
    core::{AUDIO_EXTS, DOCUMENT_EXTS, Error, FTAG_BACKUP_FILE, FTAG_FILE, IMAGE_EXTS, VIDEO_EXTS},
    walk::DirEntry,
};
use fast_glob::glob_match;
use std::{
    ffi::OsStr,
    fmt::Display,
    fs::File,
    io::Read,
    ops::Range,
    path::{Path, PathBuf},
};

/// A vector that stores small numbers of items inline to avoid heap allocation.
#[derive(Clone)]
enum SmallVec<T, const N: usize> {
    Small([T; N], usize), // array + count
    Large(Vec<T>),
}

impl<T: Default + Copy, const N: usize> SmallVec<T, N> {
    fn new() -> Self {
        Self::Small([T::default(); N], 0)
    }

    fn push(&mut self, item: T) {
        match self {
            Self::Small(arr, count) => {
                if *count < N {
                    arr[*count] = item;
                    *count += 1;
                } else {
                    // Promote to Large variant
                    let mut vec = Vec::with_capacity(N + 1);
                    vec.extend_from_slice(&arr[..]);
                    vec.push(item);
                    *self = Self::Large(vec);
                }
            }
            Self::Large(vec) => vec.push(item),
        }
    }

    fn iter(&self) -> SmallVecIter<'_, T, N> {
        match self {
            Self::Small(arr, count) => SmallVecIter::Small {
                arr,
                count: *count,
                index: 0,
            },
            Self::Large(vec) => SmallVecIter::Large { iter: vec.iter() },
        }
    }

    fn is_empty(&self) -> bool {
        match self {
            Self::Small(_, count) => *count == 0,
            Self::Large(vec) => vec.is_empty(),
        }
    }
}

enum SmallVecIter<'a, T, const N: usize> {
    Small {
        arr: &'a [T; N],
        count: usize,
        index: usize,
    },
    Large {
        iter: std::slice::Iter<'a, T>,
    },
}

impl<'a, T, const N: usize> Iterator for SmallVecIter<'a, T, N> {
    type Item = &'a T;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Small { arr, count, index } => {
                if *index < *count {
                    let item = &arr[*index];
                    *index += 1;
                    Some(item)
                } else {
                    None
                }
            }
            Self::Large { iter } => iter.next(),
        }
    }
}

pub(crate) enum Tag<'a> {
    Text(&'a str),
    Year(u16),
    Format(&'a str),
}

impl Display for Tag<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Tag::Text(t) | Tag::Format(t) => write!(f, "{t}"),
            Tag::Year(y) => write!(f, "{y}"),
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
fn infer_format_tag(input: &'_ str) -> impl Iterator<Item = Tag<'_>> + use<'_> {
    const EXT_TAG_MAP: &[(&[&str], &str)] = &[
        (VIDEO_EXTS, "video"),
        (IMAGE_EXTS, "image"),
        (AUDIO_EXTS, "audio"),
        (DOCUMENT_EXTS, "document"),
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
pub(crate) fn infer_implicit_tags(name: &'_ str) -> impl Iterator<Item = Tag<'_>> + use<'_> {
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
            None => Err(Error::InvalidPath(path.to_path_buf())),
        },
        None => Ok(""),
    }
}

/// This datastructure is responsible for finding matches between the
/// files on disk, and globs listed in the ftag file. This can be
/// reused for multiple folders to avoid reallocations.
pub(crate) struct GlobMatches {
    file_matches: Vec<SmallVec<usize, 4>>,
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
    /// knowing if a glob matches at least one file. FILES MUST BE SORTED BY
    /// NAME.
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
    let metadata = match path.metadata() {
        Ok(meta) => meta,
        Err(_) => return None,
    };
    let mut out = if metadata.is_dir() {
        PathBuf::from(path)
    } else {
        let mut out = PathBuf::from(path);
        out.pop();
        out
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
    // IMPORTANT: This MUST be the first member of the struct, because it holds
    // references to `raw_text`. Members are dropped in the order they are
    // listed here, so this ensures the references are dropped before the actual
    // data.
    parsed: DirData<'static>,
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
#[derive(Default)]
pub(crate) struct DirData<'a> {
    pub alltags: Vec<&'a str>,
    pub desc: Option<&'a str>,
    tags: Range<usize>,
    pub globs: Vec<GlobData<'a>>,
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

    pub fn reset(&mut self) {
        self.alltags.clear();
        self.desc = None;
        self.tags = 0..0;
        self.globs.clear();
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

#[derive(Debug, PartialEq)]
enum HeaderType {
    Path,
    Tags,
    Desc,
}

#[derive(Debug)]
struct Header<'a> {
    kind: HeaderType,
    content: &'a str,
}

struct HeaderIterator<'text, 'path> {
    input: &'text str,
    filepath: &'path Path,
}

impl<'text, 'path> HeaderIterator<'text, 'path> {
    fn new(input: &'text str, filepath: &'path Path) -> Result<Self, Error> {
        let input = input.trim();
        let input = if let Some(stripped) = input.strip_prefix('[') {
            stripped
        } else {
            match input.find("\n[") {
                Some(pos) => &input[(pos + 2)..], // We matched two characters.
                None => {
                    return Err(Error::CannotParseFtagFile(
                        filepath.to_path_buf(),
                        "Cannot find the first header in the file.".into(),
                    ));
                }
            }
        };
        Ok(Self {
            input: input.trim(),
            filepath,
        })
    }
}

impl<'text, 'path> Iterator for HeaderIterator<'text, 'path> {
    type Item = Result<Header<'text>, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        const HEADERS: [(&str, HeaderType); 3] = [
            ("path]", HeaderType::Path),
            ("tags]", HeaderType::Tags),
            ("desc]", HeaderType::Desc),
        ];
        if self.input.is_empty() {
            return None;
        }
        for (pat, kind) in HEADERS {
            if self.input.starts_with(pat) {
                self.input = &self.input[pat.len()..];
                let (content, next) = match self.input.find("\n[") {
                    Some(pos) => (self.input[..pos].trim(), pos + 2),
                    None => (self.input.trim(), self.input.len()),
                };
                self.input = &self.input[next..];
                return Some(Ok(Header { kind, content }));
            }
        }
        Some(Err(Error::CannotParseFtagFile(
            self.filepath.to_path_buf(),
            self.input
                .lines()
                .next()
                .unwrap_or("Unable to extract line where the error is.")
                .to_string(),
        )))
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
    // We store the data of the file we're currently parsing as:
    // (text containing a list of globs, list of tags, optional description).
    let mut current_unit: Option<(&str, Range<usize>, Option<&str>)> = None;
    // Parse file.
    for header in HeaderIterator::new(input, filepath)? {
        let header = header?;
        match header.kind {
            HeaderType::Path => {
                if let FileLoadingOptions::Skip = options.file_options {
                    break; // Stop parsing the file.
                }
                match current_unit.as_mut() {
                    Some((globs, tags, desc)) => {
                        let desc = desc.take();
                        let tags = std::mem::replace(tags, 0..0);
                        let lines = std::mem::replace(globs, header.content).lines();
                        files.extend(lines.map(|g| GlobData {
                            desc,
                            path: g.trim(),
                            tags: tags.clone(),
                        }));
                    }
                    None => current_unit = Some((header.content, 0..0, None)),
                }
            }
            HeaderType::Tags => {
                if let Some((globs, tags, _desc)) = current_unit.as_mut() {
                    if options.include_file_tags() {
                        if tags.start == tags.end {
                            // No tags found for the current unit.
                            let before = alltags.len();
                            alltags.extend(header.content.split_whitespace());
                            *tags = before..alltags.len();
                        } else {
                            return Err(Error::CannotParseFtagFile(
                                filepath.to_path_buf(),
                                format!(
                                    "The following globs have more than one 'tags' header:\n{globs}."
                                ),
                            ));
                        }
                    }
                } else if options.dir_tags {
                    if dirtags.start == dirtags.end {
                        // No directory tags found.
                        let before = alltags.len();
                        alltags.extend(header.content.split_whitespace());
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
                                    "Following globs have more than one description:\n{globs}."
                                ),
                            ));
                        } else {
                            *desc = Some(header.content);
                        }
                    }
                } else if options.dir_desc {
                    if desc.is_some() {
                        return Err(Error::CannotParseFtagFile(
                            filepath.to_path_buf(),
                            "The directory has more than one description.".into(),
                        ));
                    } else {
                        *desc = Some(header.content);
                    }
                }
            }
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
            parsed: Default::default(),
        }
    }

    /// Load the data from a .ftag file specified by the filepath.
    pub fn load<'a>(&'a mut self, filepath: &Path) -> Result<&'a DirData<'a>, Error> {
        self.raw_text.clear();
        let mut file =
            File::open(filepath).map_err(|_| Error::CannotReadStoreFile(filepath.to_path_buf()))?;
        // Reserve space based on file size to avoid reallocations
        match file.metadata() {
            Ok(metadata) => self.raw_text.reserve(metadata.len() as usize),
            Err(_) => return Err(Error::CannotReadStoreFile(filepath.to_path_buf())),
        }
        // Read contents to a string and parse.
        file.read_to_string(&mut self.raw_text)
            .map_err(|_| Error::CannotReadStoreFile(filepath.to_path_buf()))?;
        self.parsed.reset();
        let borrowed = unsafe {
            /*
             * This is safe because the returned `DirData` borrows `self`, and
             * won't let anyone modify or borrow `self` until that `DirData` is
             * dropped.
             */
            std::mem::transmute::<&'a mut DirData<'static>, &'a mut DirData<'a>>(&mut self.parsed)
        };
        load_impl(self.raw_text.trim(), filepath, &self.options, borrowed)?;
        Ok(borrowed)
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
        assert_eq!(
            infer_implicit_tags("1998_MyDirectory")
                .map(|t| t.to_string())
                .collect::<Vec<_>>(),
            vec!["1998"]
        );
        assert_eq!(
            infer_implicit_tags("1998_MyFile.pdf")
                .map(|t| t.to_string())
                .collect::<Vec<_>>(),
            vec!["1998", "document"]
        );
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

    #[test]
    fn t_parse_complete_ftag_file() {
        let input = r#"
[tags]
dir_tag1 dir_tag2

[desc]
Directory description

[path]
*.jpg
file.txt

[tags]
image text

[desc]
Mixed file types

[path]
video/*

[tags]
video media
"#;
        let mut data = DirData::default();
        let options = LoaderOptions::new(
            true,
            true,
            FileLoadingOptions::Load {
                file_tags: true,
                file_desc: true,
            },
        );
        load_impl(input, Path::new("dummy_file_path"), &options, &mut data).unwrap();
        assert_eq!(data.tags(), &["dir_tag1", "dir_tag2"]);
        assert_eq!(data.desc, Some("Directory description"));
        assert_eq!(data.globs.len(), 3);
        assert_eq!(data.globs[0].path, "*.jpg");
        assert_eq!(data.globs[1].path, "file.txt");
        assert_eq!(data.globs[0].tags(&data.alltags), &["image", "text"]);
        assert_eq!(data.globs[0].desc, Some("Mixed file types"));
        assert_eq!(data.globs[2].path, "video/*");
        assert_eq!(data.globs[2].tags(&data.alltags), &["video", "media"]);
    }

    #[test]
    fn t_parse_with_loading_options() {
        let input = r#"
[tags]
dir_tag

[desc]
Directory description

[path]
file.txt

[tags]
file_tag

[desc]
File description
"#;
        // Test directory-only loading
        let mut data = DirData::default();
        let options = LoaderOptions::new(true, true, FileLoadingOptions::Skip);
        load_impl(input, Path::new("dummy_file_path"), &options, &mut data).unwrap();
        assert_eq!(data.tags(), &["dir_tag"]);
        assert_eq!(data.desc, Some("Directory description"));
        assert_eq!(data.globs.len(), 0);
        // Test file tags only
        data.reset();
        let options = LoaderOptions::new(
            false,
            false,
            FileLoadingOptions::Load {
                file_tags: true,
                file_desc: false,
            },
        );
        load_impl(input, Path::new("dummy_file_path"), &options, &mut data).unwrap();
        assert_eq!(data.tags(), &[] as &[&str]);
        assert_eq!(data.desc, None);
        assert_eq!(data.globs.len(), 1);
        assert_eq!(data.globs[0].tags(&data.alltags), &["file_tag"]);
        assert_eq!(data.globs[0].desc, None);
    }

    #[test]
    fn t_whitespace_and_empty_sections() {
        let input = r#"


[tags]
  tag1   tag2

[desc]

[path]
  file.txt

[tags]


"#;
        let mut data = DirData::default();
        let options = LoaderOptions::new(
            true,
            true,
            FileLoadingOptions::Load {
                file_tags: true,
                file_desc: true,
            },
        );
        load_impl(input, Path::new("dummy_file_path"), &options, &mut data).unwrap();
        assert_eq!(data.tags(), &["tag1", "tag2"]);
        assert_eq!(data.desc, Some(""));
        assert_eq!(data.globs.len(), 1);
        assert_eq!(data.globs[0].path, "file.txt");
        assert_eq!(data.globs[0].tags(&data.alltags), &[] as &[&str]);
    }

    #[test]
    fn t_error_conditions() {
        let mut data = DirData::default();
        let options = LoaderOptions::new(true, true, FileLoadingOptions::Skip);
        // No headers
        let result = load_impl(
            "plain text",
            Path::new("dummy_file_path"),
            &options,
            &mut data,
        );
        assert!(matches!(result, Err(Error::CannotParseFtagFile(_, _))));
        // Multiple directory tags
        data.reset();
        let input = "[tags]\ntag1\n[tags]\ntag2";
        let result = load_impl(input, Path::new("dummy_file_path"), &options, &mut data);
        assert!(matches!(result, Err(Error::CannotParseFtagFile(_, _))));
        // Multiple file tags for same group
        data.reset();
        let options = LoaderOptions::new(
            false,
            false,
            FileLoadingOptions::Load {
                file_tags: true,
                file_desc: false,
            },
        );
        let input = "[path]\nfile.txt\n[tags]\ntag1\n[tags]\ntag2";
        let result = load_impl(input, Path::new("dummy_file_path"), &options, &mut data);
        assert!(matches!(result, Err(Error::CannotParseFtagFile(_, _))));
    }

    #[test]
    fn t_edge_cases_and_boundary_conditions() {
        let mut data = DirData::default();
        let options = LoaderOptions::new(
            true,
            true,
            FileLoadingOptions::Load {
                file_tags: true,
                file_desc: true,
            },
        );
        // Header at end of file without trailing newline
        let input = "[tags]\ntag1 tag2\n[desc]\nend description";
        load_impl(input, Path::new("dummy_file_path"), &options, &mut data).unwrap();
        assert_eq!(data.tags(), &["tag1", "tag2"]);
        assert_eq!(data.desc, Some("end description"));

        // Empty content sections and multiple consecutive newlines
        data.reset();
        let input = "[tags]\n\n\n[desc]\n\n[path]\n\n\nfile.txt\n\n";
        load_impl(input, Path::new("dummy_file_path"), &options, &mut data).unwrap();
        assert_eq!(data.tags(), &[] as &[&str]);
        assert_eq!(data.desc, Some(""));
        assert_eq!(data.globs.len(), 1);
        assert_eq!(data.globs[0].path, "file.txt");

        // File ending with partial header pattern - trailing [ terminates content
        data.reset();
        let input = "[tags]\ntag1\nsome text ending with\n[";
        load_impl(input, Path::new("dummy_file_path"), &options, &mut data).unwrap();
        assert_eq!(data.tags(), &["tag1", "some", "text", "ending", "with"]);

        // Unknown header should cause error
        data.reset();
        let input = "[tags]\ntag1\n[unknown]\ncontent";
        let result = load_impl(input, Path::new("dummy_file_path"), &options, &mut data);
        assert!(matches!(result, Err(Error::CannotParseFtagFile(_, _))));
    }

    #[test]
    fn t_smallvec_basic_operations() {
        // Test empty SmallVec
        let mut sv: SmallVec<usize, 4> = SmallVec::new();
        assert!(sv.is_empty());
        assert_eq!(sv.iter().count(), 0);
        // Test small capacity (stays on stack)
        sv.push(1);
        sv.push(2);
        sv.push(3);
        sv.push(4);
        assert!(!sv.is_empty());
        let items: Vec<usize> = sv.iter().copied().collect();
        assert_eq!(items, vec![1, 2, 3, 4]);
        // Verify still Small variant by checking we haven't allocated
        match sv {
            SmallVec::Small(_, count) => assert_eq!(count, 4),
            SmallVec::Large(_) => panic!("Should still be Small variant"),
        }
    }

    #[test]
    fn t_smallvec_promotion_to_large() {
        let mut sv: SmallVec<usize, 3> = SmallVec::new();
        // Fill to capacity
        sv.push(10);
        sv.push(20);
        sv.push(30);
        // Should still be Small
        match sv {
            SmallVec::Small(_, count) => assert_eq!(count, 3),
            SmallVec::Large(_) => panic!("Should still be Small variant"),
        }
        // Push one more to trigger promotion
        sv.push(40);
        // Should now be Large
        match sv {
            SmallVec::Small(_, _) => panic!("Should have promoted to Large variant"),
            SmallVec::Large(ref vec) => assert_eq!(vec.len(), 4),
        }
        // Verify all items are still accessible
        let items: Vec<usize> = sv.iter().copied().collect();
        assert_eq!(items, vec![10, 20, 30, 40]);
        // Test continued pushing to Large variant
        sv.push(50);
        sv.push(60);
        let items: Vec<usize> = sv.iter().copied().collect();
        assert_eq!(items, vec![10, 20, 30, 40, 50, 60]);
    }

    #[test]
    fn t_smallvec_edge_cases() {
        // Test with capacity 0 (always promotes)
        let mut sv: SmallVec<i32, 0> = SmallVec::new();
        assert!(sv.is_empty());
        sv.push(42);
        match sv {
            SmallVec::Small(_, _) => panic!("Should have promoted immediately"),
            SmallVec::Large(ref vec) => assert_eq!(vec[0], 42),
        }
        // Test with capacity 1
        let mut sv: SmallVec<char, 1> = SmallVec::new();
        sv.push('a');
        assert_eq!(sv.iter().copied().collect::<Vec<_>>(), vec!['a']);
        sv.push('b'); // Should promote
        match sv {
            SmallVec::Small(_, _) => panic!("Should have promoted"),
            SmallVec::Large(_) => {} // Expected
        }
        assert_eq!(sv.iter().copied().collect::<Vec<_>>(), vec!['a', 'b']);
        // Test clone functionality
        let sv_clone = sv.clone();
        assert_eq!(
            sv.iter().copied().collect::<Vec<_>>(),
            sv_clone.iter().copied().collect::<Vec<_>>()
        );
    }
}
