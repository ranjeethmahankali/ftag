use glob_match::glob_match;
use serde::Deserialize;
use std::{
    ffi::OsStr,
    fs::File,
    io::BufReader,
    ops::Range,
    path::{Path, PathBuf},
};

use crate::{
    core::{FstoreError, FSTORE},
    walk::DirEntry,
};

fn infer_year_range_bytes(mut input: &[u8]) -> Option<Range<u16>> {
    use nom::{
        bytes::complete::{tag, take_while_m_n},
        character::is_digit,
        error::Error,
        IResult, ParseTo,
    };
    type Result<'a> = IResult<&'a [u8], &'a [u8], Error<&'a [u8]>>;
    let result: Result = take_while_m_n(4, 4, is_digit)(input);
    let first: u16 = match result {
        Ok((i, o)) if o.len() > 3 => {
            input = i;
            o.parse_to()?
        }
        _ => return None,
    };
    let result: Result = tag("_")(input);
    match result {
        Ok((i, _o)) => input = i,
        Err(_) => return Some(first..(first + 1)),
    }
    let result: Result = take_while_m_n(4, 4, is_digit)(input);
    if let Ok((_i, o)) = result {
        let second: u16 = o.parse_to().unwrap_or(first);
        return Some(first..(second + 1));
    }
    let result: Result = tag("to_")(input);
    match result {
        Ok((i, _o)) => input = i,
        Err(_) => return Some(first..(first + 1)),
    }
    let result: Result = take_while_m_n(4, 4, is_digit)(input);
    if let Ok((_i, o)) = result {
        let second: u16 = o.parse_to().unwrap_or(first);
        return Some(first..(second + 1));
    }
    return None;
}

fn infer_year_range_os_str(nameopt: Option<&OsStr>) -> Option<Range<u16>> {
    match nameopt {
        Some(val) => infer_year_range_bytes(val.as_encoded_bytes()),
        None => None,
    }
}

fn infer_year_range_str(name: &str) -> Option<Range<u16>> {
    return infer_year_range_bytes(name.as_bytes());
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

#[derive(Deserialize)]
pub(crate) struct FileData {
    pub desc: Option<String>,
    pub path: String,
    pub tags: Option<Vec<String>>,
}

#[derive(Deserialize)]
pub(crate) struct DirData {
    pub desc: Option<String>,
    pub tags: Option<Vec<String>>,
    pub files: Option<Vec<FileData>>,
}

impl DirData {
    fn infer_implicit_tags(&mut self, dname: Option<&OsStr>) {
        let iter = implicit_tags_os_str(dname);
        match &mut self.tags {
            Some(tags) => tags.extend(iter),
            None => self.tags = Some(iter.collect()),
        }
        if let Some(files) = &mut self.files {
            for file in files.iter_mut() {
                file.infer_implicit_tags();
            }
        }
    }
}

impl FileData {
    fn infer_implicit_tags(&mut self) {
        let iter = implicit_tags_str(&self.path);
        match &mut self.tags {
            Some(tags) => tags.extend(iter),
            None => self.tags = Some(iter.collect()),
        }
    }
}

pub(crate) fn read_store_file(storefile: PathBuf) -> Result<DirData, FstoreError> {
    let mut data: DirData = serde_yaml::from_reader(BufReader::new(
        File::open(&storefile).map_err(|_| FstoreError::CannotReadStoreFile(storefile.clone()))?,
    ))
    .map_err(|e| FstoreError::CannotParseYaml(format!("{:?}\n{:?}", storefile, e)))?;
    if let Some(parent) = storefile.parent() {
        data.infer_implicit_tags(parent.file_name());
    }
    return Ok(data);
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
