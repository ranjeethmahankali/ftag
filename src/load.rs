use glob_match::glob_match;
use std::{
    ffi::OsStr,
    fs::File,
    io::Read,
    ops::Range,
    path::{Path, PathBuf},
};

use crate::{
    core::{Error, FSTORE},
    walk::DirEntry,
};

fn infer_year_range_os_str(nameopt: Option<&OsStr>) -> Option<Range<u16>> {
    match nameopt {
        Some(val) => match val.to_str() {
            Some(strval) => infer_year_range_str(strval),
            None => None,
        },
        None => None,
    }
}

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
    return Some(first..(first + 1));
}

pub(crate) fn implicit_tags_os_str(name: Option<&OsStr>) -> impl Iterator<Item = String> {
    infer_year_range_os_str(name)
        .unwrap_or(0..0)
        .map(|y| y.to_string())
}

pub(crate) fn implicit_tags_str(name: &str) -> impl Iterator<Item = String> {
    infer_year_range_str(name)
        .unwrap_or(0..0)
        .map(|y| y.to_string())
}

pub(crate) struct GlobMatches {
    table: Vec<bool>,
    glob_matches: Vec<Option<usize>>, // Direct map from glob to file.
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
    fn row(&self, gi: usize) -> &[bool] {
        let start = gi * self.num_files;
        &self.table[start..(start + self.num_files)]
    }

    fn row_and_match(&mut self, gi: usize) -> (&mut [bool], &mut Option<usize>) {
        let start = gi * self.num_files;
        (
            &mut self.table[start..(start + self.num_files)],
            &mut self.glob_matches[gi],
        )
    }

    pub fn find_matches(&mut self, files: &[DirEntry], globs: &[FileData], short_circuit: bool) {
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
                continue;
            }
            for (fi, f) in files.iter().enumerate() {
                if let Some(fstr) = f.name().to_str() {
                    if glob_match(&g.path, fstr) {
                        row[fi] = true;
                        if short_circuit {
                            break;
                        }
                    }
                }
            }
        }
    }

    pub fn matched_globs<'a>(&'a self, file_index: usize) -> impl Iterator<Item = usize> + 'a {
        (0..self.num_globs).filter(move |gi| self.row(*gi)[file_index])
    }

    pub fn is_glob_matched(&self, glob_index: usize) -> bool {
        self.row(glob_index).iter().any(|m| *m)
    }
}

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

pub(crate) struct Loader {
    raw_text: String,
    options: LoaderOptions,
}

pub(crate) struct FileData<'a> {
    pub desc: Option<&'a str>,
    pub path: &'a str,
    pub tags: Vec<&'a str>,
}

pub(crate) struct DirData<'a> {
    pub desc: Option<&'a str>,
    pub tags: Vec<&'a str>,
    pub files: Vec<FileData<'a>>,
}

pub(crate) enum FileLoadingOptions {
    Skip,
    Load { file_tags: bool, file_desc: bool },
}

pub(crate) struct LoaderOptions {
    dir_tags: bool,
    dir_desc: bool,
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

    pub fn include_file_desc(&self) -> bool {
        match self.options.file_options {
            FileLoadingOptions::Skip => false,
            FileLoadingOptions::Load {
                file_tags: _,
                file_desc,
            } => file_desc,
        }
    }

    pub fn include_file_tags(&self) -> bool {
        match self.options.file_options {
            FileLoadingOptions::Skip => false,
            FileLoadingOptions::Load {
                file_tags,
                file_desc: _,
            } => file_tags,
        }
    }

    pub fn load<'a>(&'a mut self, storefile: &Path) -> Result<DirData<'a>, Error> {
        self.raw_text.clear();
        File::open(&storefile)
            .map_err(|_| Error::CannotReadStoreFile(storefile.to_path_buf()))?
            .read_to_string(&mut self.raw_text)
            .map_err(|_| Error::CannotReadStoreFile(storefile.to_path_buf()))?;
        let mut tags: Vec<&str> = Vec::new();
        let mut desc: Option<&str> = None;
        let mut files: Vec<FileData<'a>> = Vec::new();
        let mut curfile: Option<FileData<'a>> = None;
        let mut input = self.raw_text.trim();
        if !input.starts_with('[') {
            return Err(Error::CannotParseYaml(
                storefile.to_path_buf(),
                "File does not begin with a header.".into(),
            ));
        }
        while let Some(start) = input.find('[') {
            // Walk the text one header at a time.
            let start = start + 1;
            let end = input.find(']').ok_or(Error::CannotParseYaml(
                storefile.to_path_buf(),
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
            if header == "desc" {
                if let Some(file) = &mut curfile {
                    if !self.include_file_desc() {
                        continue;
                    }
                    let FileData {
                        path,
                        tags: _,
                        desc,
                    } = file;
                    if let Some(_) = desc {
                        return Err(Error::CannotParseYaml(
                            storefile.to_path_buf(),
                            format!("'{}' has more than one description.", path),
                        ));
                    } else {
                        *desc = Some(content);
                    }
                } else {
                    if !self.options.dir_desc {
                        continue;
                    }
                    if let Some(_) = &mut desc {
                        return Err(Error::CannotParseYaml(
                            storefile.to_path_buf(),
                            "The directory has more than one description.".into(),
                        ));
                    } else {
                        desc = Some(content);
                    }
                }
            } else if header == "tags" {
                if let Some(file) = &mut curfile {
                    if !self.include_file_tags() {
                        continue;
                    }
                    let FileData {
                        path,
                        tags,
                        desc: _,
                    } = file;
                    if tags.is_empty() {
                        tags.extend(content.split_whitespace().map(|w| w.trim()));
                    } else {
                        return Err(Error::CannotParseYaml(
                            storefile.to_path_buf(),
                            format!("'{}' has more than one 'tags' header.", path),
                        ));
                    }
                } else {
                    if !self.options.dir_tags {
                        continue;
                    }
                    if tags.is_empty() {
                        tags.extend(content.split_whitespace().map(|w| w.trim()));
                    } else {
                        return Err(Error::CannotParseYaml(
                            storefile.to_path_buf(),
                            "The directory has more than one 'tags' header.".into(),
                        ));
                    }
                }
            } else if header == "path" {
                if let FileLoadingOptions::Skip = self.options.file_options {
                    break; // Stop parsing the file.
                }
                let newfile: FileData = FileData {
                    desc: None,
                    path: content.trim(),
                    tags: Vec::new(),
                };
                if let Some(prev) = std::mem::replace(&mut curfile, Some(newfile)) {
                    files.push(prev);
                }
            } else {
                return Err(Error::CannotParseYaml(
                    storefile.to_path_buf(),
                    format!("Unrecognized header: {header}"),
                ));
            }
        }
        if let Some(file) = curfile {
            files.push(file);
        }
        return Ok(DirData { desc, tags, files });
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use std::ffi::OsString;

    #[test]
    fn t_infer_year_range() {
        let inputs = vec![OsString::from("2021_to_2023"), OsString::from("2021_2023")];
        let expected = vec!["2021", "2022", "2023"];
        for input in inputs {
            let actual: Vec<_> = implicit_tags_os_str(Some(&input)).collect();
            assert_eq!(actual, expected);
        }
        let inputs = vec![
            OsString::from("1998_MyDirectory"),
            OsString::from("1998_MyFile.pdf"),
        ];
        let expected = vec!["1998"];
        for input in inputs {
            let actual: Vec<_> = implicit_tags_os_str(Some(&input)).collect();
            assert_eq!(actual, expected);
        }
    }
}
