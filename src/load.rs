use glob_match::glob_match;
use std::{
    fmt::Display,
    fs::File,
    io::Read,
    ops::Range,
    path::{Path, PathBuf},
};

use crate::{
    core::{Error, FTAG_FILE},
    walk::DirEntry,
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
fn infer_format_tag(input: &str) -> impl Iterator<Item = String> + '_ {
    const EXT_TAG_MAP: &[(&[&str], &str)] = &[
        (&[".mov", ".flv", ".mp4", ".3gp"], "video"),
        (&[".png", ".jpg", ".jpeg", ".bmp", ".webp"], "image"),
    ];
    EXT_TAG_MAP.iter().filter_map(|(exts, tag)| {
        if exts
            .iter()
            .any(|ext| input[input.len() - ext.len()..].eq_ignore_ascii_case(ext))
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
    glob_matches: Vec<Option<usize>>, // Direct 1 to 1 map from glob to file.
    num_files: usize,
    num_globs: usize,
}

impl GlobMatches {
    pub fn new() -> GlobMatches {
        GlobMatches {
            table: Vec::new(),
            glob_matches: Vec::new(),
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
    fn row_and_match(&mut self, gi: usize) -> (&mut [bool], &mut Option<usize>) {
        let start = gi * self.num_files;
        (
            &mut self.table[start..(start + self.num_files)],
            &mut self.glob_matches[gi],
        )
    }

    /// Populate this struct with matches from a new set of `files` and
    /// `globs`. If `short_circuit_globs` is true, then each glob will be
    /// matched with at most 1 file on disk. This is useful when you're not
    /// interested in matching all possible files, but only interested in
    /// knowing if a glob matches at least one file.
    pub fn find_matches(
        &mut self,
        files: &[DirEntry],
        globs: &[FileData],
        short_circuit_globs: bool,
    ) {
        self.num_files = files.len();
        self.num_globs = globs.len();
        self.table.clear();
        self.table.resize(files.len() * globs.len(), false);
        self.glob_matches.clear();
        self.glob_matches.resize(globs.len(), None);
        // Find perfect matches.
        for (gi, g) in globs.iter().enumerate() {
            let (row, gmatch) = self.row_and_match(gi);
            for (fi, f) in files.iter().enumerate() {
                if let Some(fstr) = f.name().to_str() {
                    if g.path == fstr {
                        row[fi] = true;
                        *gmatch = Some(fi);
                        break;
                    }
                }
            }
            if gmatch.is_some() {
                // If a glob matches a file directly, then we don't need to
                // search any further.
                continue;
            }
            for (fi, f) in files.iter().enumerate() {
                if let Some(fstr) = f.name().to_str() {
                    if glob_match(g.path, fstr) {
                        row[fi] = true;
                        if short_circuit_globs {
                            break;
                        }
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
pub fn get_store_path<const MUST_EXIST: bool>(path: &Path) -> Option<PathBuf> {
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

/// Loads and parses an ftag file. Reuse this to avoid allocations.
pub(crate) struct Loader {
    raw_text: String,
    options: LoaderOptions,
}

/// Data in an ftaXOg file, corresponding to one file / glob.
pub(crate) struct FileData<'a> {
    pub desc: Option<&'a str>,
    pub path: &'a str,
    pub tags: Vec<&'a str>,
}

/// Data from an ftag file.
pub(crate) struct DirData<'a> {
    pub desc: Option<&'a str>,
    pub tags: Vec<&'a str>,
    pub files: Vec<FileData<'a>>,
}

pub(crate) enum FTagUnit<'a> {
    File {
        globs: Vec<&'a str>,
        tags: Vec<&'a str>,
        desc: Option<&'a str>,
    },
    Dir {
        tags: Vec<&'a str>,
        desc: Option<&'a str>,
    },
}

impl<'a> FTagUnit<'a> {
    fn is_empty(&self) -> bool {
        match self {
            FTagUnit::File { globs, tags, desc } => {
                globs.is_empty() && tags.is_empty() && desc.is_none()
            }
            FTagUnit::Dir { tags, desc } => tags.is_empty() && desc.is_none(),
        }
    }
}

impl<'a> Display for FTagUnit<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FTagUnit::File { globs, tags, desc } => {
                writeln!(f, "[path]")?;
                for &g in globs {
                    writeln!(f, "{}", g)?;
                }
                if !tags.is_empty() {
                    writeln!(f, "[tags]")?;
                    tags.iter().try_fold(0usize, |len, &t| {
                        Ok(if len > 80 {
                            writeln!(f, "{}", t)?;
                            0usize
                        } else {
                            write!(f, "{} ", t)?;
                            len + t.len() + 1
                        })
                    })?;
                }
                if let Some(desc) = desc {
                    writeln!(f, "[desc]")?;
                    writeln!(f, "{}", desc)?;
                }
                writeln!(f, "")
            }
            FTagUnit::Dir { tags, desc } => {
                if !tags.is_empty() {
                    writeln!(f, "[tags]")?;
                    tags.iter().try_fold(0usize, |len, &t| {
                        Ok(if len > 80 {
                            writeln!(f, "{}", t)?;
                            0usize
                        } else {
                            write!(f, "{} ", t)?;
                            len + t.len() + 1
                        })
                    })?;
                }
                if let Some(desc) = desc {
                    writeln!(f, "[desc]")?;
                    writeln!(f, "{}", desc)?;
                }
                writeln!(f, "")
            }
        }
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

    pub fn visit_units<'a, V: FnMut(FTagUnit<'a>) -> (), P: AsRef<Path>>(
        &'a mut self,
        filepath: P,
        mut visitor: V,
    ) -> Result<(), Error> {
        self.raw_text.clear();
        let filepath = filepath.as_ref();
        File::open(filepath)
            .map_err(|_| Error::CannotReadStoreFile(filepath.to_path_buf()))?
            .read_to_string(&mut self.raw_text)
            .map_err(|_| Error::CannotReadStoreFile(filepath.to_path_buf()))?;
        let mut current = FTagUnit::Dir {
            tags: Vec::new(),
            desc: None,
        };
        let mut input = self.raw_text.trim();
        if !input.starts_with('[') {
            return Err(Error::CannotParseFtagFile(
                filepath.to_path_buf(),
                "File does not begin with a header.".into(),
            ));
        }
        while let Some(start) = input.find('[') {
            // Walk the text one header at a time.
            let start = start + 1;
            let end = input.find(']').ok_or(Error::CannotParseFtagFile(
                filepath.to_path_buf(),
                "Header doesn't terminate".into(),
            ))?;
            let header = &input[start..end];
            let start = end + 1;
            input = &input[start..];
            let content = match input.find("\n[") {
                Some(end) => {
                    let c = &input[..end];
                    input = &input[end..];
                    c
                }
                None => {
                    let c = input;
                    input = "";
                    c
                }
            }
            .trim();
            input = input.trim();
            match header {
                "desc" => match &mut current {
                    FTagUnit::File {
                        globs,
                        tags: _,
                        desc,
                    } if self.include_file_desc() => {
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
                    FTagUnit::Dir { tags: _, desc } if self.options.dir_desc => match desc {
                        Some(_) => {
                            return Err(Error::CannotParseFtagFile(
                                filepath.to_path_buf(),
                                "The directory has more than one description.".into(),
                            ))
                        }
                        None => *desc = Some(content),
                    },
                    _ => continue,
                },
                "tags" => match &mut current {
                    FTagUnit::File {
                        globs,
                        tags,
                        desc: _,
                    } if self.include_file_tags() => {
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
                    FTagUnit::Dir { tags, desc: _ } if self.options.dir_tags => {
                        if tags.is_empty() {
                            tags.extend(content.split_whitespace().map(|w| w.trim()));
                        } else {
                            return Err(Error::CannotParseFtagFile(
                                filepath.to_path_buf(),
                                "The directory has more than one 'tags' header.".into(),
                            ));
                        }
                    }
                    _ => continue,
                },
                "path" => {
                    if let FileLoadingOptions::Skip = self.options.file_options {
                        break; // Stop parsing the file.
                    }
                    visitor(std::mem::replace(
                        &mut current,
                        FTagUnit::File {
                            globs: content.lines().collect(),
                            tags: Vec::new(),
                            desc: None,
                        },
                    ));
                }
                _ => {
                    return Err(Error::CannotParseFtagFile(
                        filepath.to_path_buf(),
                        format!("Unrecognized header: {header}"),
                    ))
                }
            }
        }
        if !current.is_empty() {
            visitor(current);
        }
        Ok(())
    }

    /// Load the data from a .ftag file specified by the filepath.
    pub fn load<'a, P: AsRef<Path>>(&'a mut self, filepath: P) -> Result<DirData<'a>, Error> {
        let mut tags: Vec<&'a str> = Vec::new();
        let mut desc: Option<&'a str> = None;
        let mut files: Vec<FileData<'a>> = Vec::new();
        self.visit_units(filepath, |unit: FTagUnit<'a>| match unit {
            FTagUnit::File {
                mut globs,
                tags,
                desc,
            } => {
                files.extend(globs.drain(..).map(|g| FileData {
                    desc: desc.clone(),
                    path: g,
                    tags: tags.clone(),
                }));
            }
            FTagUnit::Dir { tags: t, desc: d } => {
                tags = t;
                desc = d;
            }
        })?;
        Ok(DirData { desc, tags, files })
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
    fn t_temp() {
        let mut loader = Loader::new(LoaderOptions {
            dir_tags: true,
            dir_desc: true,
            file_options: FileLoadingOptions::Load {
                file_tags: true,
                file_desc: true,
            },
        });
        loader
            .visit_units(
                "/home/rnjth94/archived/pictures/1992_laxman_nirmala_trip/.ftag",
                |unit| println!("{}", unit),
            )
            .unwrap();
    }
}
