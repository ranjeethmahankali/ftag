use crate::{
    core::{
        get_filenames, get_relative_path, get_store_path, glob_filter, read_store_file,
        FstoreError, WalkDirectories,
    },
    filter::{Filter, TagIndex, TagMaker},
};
use hashbrown::HashMap;
use serde::Deserialize;
use std::path::{Path, PathBuf};

fn safe_set_flag(flags: &mut Vec<bool>, index: usize) {
    if index >= flags.len() {
        flags.resize(index + 1, false);
    }
    flags[index] = true;
}

pub(crate) fn safe_get_flag(flags: &Vec<bool>, index: usize) -> bool {
    *flags.get(index).unwrap_or(&false)
}

pub(crate) struct TagTable {
    root: PathBuf,
    index_map: HashMap<String, usize>,
    table: HashMap<PathBuf, Vec<bool>>,
}

struct InheritedTags {
    tag_indices: Vec<usize>,
    offsets: Vec<usize>,
    path: Option<PathBuf>,
}

impl InheritedTags {
    fn count_components(oldpath: &Path, newpath: &Path) -> (usize, usize, usize) {
        let mut before = 0;
        let mut after = 0;
        let mut common = 0;
        let mut olditer = oldpath.components();
        let mut newiter = newpath.components();
        loop {
            match (olditer.next(), newiter.next()) {
                (None, None) => break,
                (None, Some(_)) => after += 1,
                (Some(_), None) => before += 1,
                (Some(l), Some(r)) => {
                    before += 1;
                    after += 1;
                    if l == r {
                        common += 1;
                    }
                }
            }
        }
        return (before, after, common);
    }

    fn update(&mut self, newpath: &Path) -> Result<(), FstoreError> {
        match &self.path {
            Some(path) => {
                let (before, after, common) = Self::count_components(path, newpath);
                if before == common && after == before + 1 {
                    self.offsets.push(self.tag_indices.len());
                } else if before > common && after == common + 1 {
                    let mut marker = self.tag_indices.len();
                    for _ in 0..(before - common) {
                        marker = self
                            .offsets
                            .pop()
                            .ok_or(FstoreError::TagInheritanceFailed)?;
                    }
                    self.tag_indices.truncate(marker);
                    self.offsets.push(marker);
                } else {
                    return Err(FstoreError::DirectoryTraversalFailed);
                }
            }
            None => {
                self.offsets.push(self.tag_indices.len());
            }
        };
        self.path = Some(PathBuf::from(newpath));
        return Ok(());
    }
}

impl TagTable {
    fn query<'a>(
        &'a self,
        filter: Filter<TagIndex>,
    ) -> impl Iterator<Item = std::path::Display<'a>> {
        self.table.iter().filter_map(move |(path, flags)| {
            if filter.evaluate(flags) {
                Some(path.display())
            } else {
                None
            }
        })
    }

    pub(crate) fn from_dir(dirpath: PathBuf) -> Result<Self, FstoreError> {
        // These structs are locally defined because their only
        // purpose is to use with serde_yaml to extract relevant
        // information from the YAML files.
        #[derive(Deserialize)]
        struct FileData {
            path: String,
            tags: Option<Vec<String>>,
        }
        #[derive(Deserialize)]
        struct DirData {
            tags: Option<Vec<String>>,
            files: Option<Vec<FileData>>,
        }
        let mut table = TagTable {
            root: dirpath,
            index_map: HashMap::new(),
            table: HashMap::new(),
        };
        let mut num_tags: usize = 0;
        let mut inherited = InheritedTags {
            tag_indices: Vec::new(),
            offsets: Vec::new(),
            path: None,
        };
        let rootdir = table.root.clone(); // We'll need this copy later.
        let mut walker = WalkDirectories::from(table.root.clone())?;
        while let Some((_depth, curpath, children)) = walker.next() {
            inherited.update(&curpath)?;
            // Deserialize yaml without copy.
            let DirData { tags, files } = {
                match get_store_path::<true>(&curpath) {
                    Some(path) => read_store_file(path)?,
                    None => continue,
                }
            };
            // Push store tags.
            if let Some(tags) = tags {
                for tag in tags {
                    inherited.tag_indices.push(Self::get_tag_index(
                        &tag,
                        &mut table.index_map,
                        &mut num_tags,
                    ));
                }
            }
            if let Some(files) = files {
                for FileData {
                    path: pattern,
                    tags,
                } in files
                {
                    for fpath in get_filenames(children)
                        .filter(glob_filter(&pattern))
                        .filter_map(|fname| get_relative_path(&curpath, fname, &rootdir))
                    {
                        table.add_file(fpath, &tags, &mut num_tags, &inherited.tag_indices);
                    }
                }
            }
        }
        return Ok(table);
    }

    fn get_tag_index(tag: &String, map: &mut HashMap<String, usize>, counter: &mut usize) -> usize {
        *(map.entry(tag.clone()).or_insert({
            let index = *counter;
            *counter += 1;
            index
        }))
    }

    fn add_file(
        &mut self,
        path: PathBuf,
        tags: &Option<Vec<String>>,
        num_tags: &mut usize,
        inherited: &Vec<usize>,
    ) {
        let flags = self.table.entry(path).or_insert(Vec::new());
        if let Some(tags) = tags {
            flags.reserve(flags.len() + tags.len());
            for tag in tags {
                let i = Self::get_tag_index(tag, &mut self.index_map, num_tags);
                safe_set_flag(flags, i);
            }
        }
        for i in inherited {
            safe_set_flag(flags, *i);
        }
    }
}

impl TagMaker<TagIndex> for TagTable {
    fn create_tag(&self, input: &str) -> TagIndex {
        TagIndex {
            value: match self.index_map.get(&input.to_string()) {
                Some(i) => Some(*i),
                None => None,
            },
        }
    }
}

pub(crate) fn run_query(dirpath: PathBuf, filter: &String) -> Result<(), FstoreError> {
    let table = TagTable::from_dir(dirpath)?;
    let filter = Filter::<TagIndex>::parse(filter.as_str(), &table)
        .map_err(|e| FstoreError::InvalidFilter(e))?;
    for path in table.query(filter) {
        println!("{}", path);
    }
    return Ok(());
}
