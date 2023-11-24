use crate::{
    core::{
        get_filenames, get_relative_path, get_store_path, glob_filter, implicit_tags,
        read_store_file, FstoreError, WalkDirectories,
    },
    filter::{Filter, TagIndex, TagMaker},
};
use hashbrown::HashMap;
use serde::Deserialize;
use std::path::PathBuf;

fn safe_set_flag(flags: &mut Vec<bool>, index: usize) {
    if index >= flags.len() {
        flags.resize(index + 1, false);
    }
    flags[index] = true;
}

pub(crate) fn safe_get_flag(flags: &Vec<bool>, index: usize) -> bool {
    *flags.get(index).unwrap_or(&false)
}

struct InheritedTags {
    tag_indices: Vec<usize>,
    offsets: Vec<usize>,
    depth: usize,
}

impl InheritedTags {
    fn update(&mut self, newdepth: usize) -> Result<(), FstoreError> {
        if self.depth + 1 == newdepth {
            self.offsets.push(self.tag_indices.len());
        } else if self.depth >= newdepth {
            let mut marker = self.tag_indices.len();
            for _ in 0..(self.depth + 1 - newdepth) {
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
        self.depth = newdepth;
        return Ok(());
    }
}

pub(crate) struct TagTable {
    root: PathBuf,
    index_map: HashMap<String, usize>,
    table: HashMap<PathBuf, Vec<bool>>,
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
            depth: 0,
        };
        let rootdir = table.root.clone(); // We'll need this copy later.
        let mut walker = WalkDirectories::from(table.root.clone())?;
        while let Some((depth, curpath, children)) = walker.next() {
            inherited.update(depth)?;
            // Deserialize yaml without copy.
            let DirData { tags, files } = {
                match get_store_path::<true>(&curpath) {
                    Some(path) => read_store_file(path)?,
                    None => continue,
                }
            };
            // Push directory tags.
            if let Some(tags) = tags {
                for tag in tags {
                    inherited.tag_indices.push(Self::get_tag_index(
                        &tag,
                        &mut table.index_map,
                        &mut num_tags,
                    ));
                }
            }
            // Implicit directory tags.
            for tag in implicit_tags(curpath.file_name()) {
                inherited.tag_indices.push(Self::get_tag_index(
                    &tag,
                    &mut table.index_map,
                    &mut num_tags,
                ));
            }
            // Process all files in the directory.
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
        let size = map.len();
        let entry = *(map.entry(tag.clone()).or_insert(size));
        *counter = map.len();
        return entry;
    }

    fn add_file(
        &mut self,
        path: PathBuf,
        tags: &Option<Vec<String>>,
        num_tags: &mut usize,
        inherited: &Vec<usize>,
    ) {
        let impltags = implicit_tags(path.file_name());
        let flags = self.table.entry(path).or_insert(Vec::new());
        // Set the file's explicit tags.
        if let Some(tags) = tags {
            flags.reserve(flags.len() + tags.len());
            for tag in tags {
                safe_set_flag(
                    flags,
                    Self::get_tag_index(tag, &mut self.index_map, num_tags),
                );
            }
        }
        // Implicit tags.
        for tag in impltags {
            safe_set_flag(
                flags,
                Self::get_tag_index(&tag, &mut self.index_map, num_tags),
            );
        }
        // Set inherited tags.
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

pub(crate) struct BoolTable {
    data: Box<[bool]>, // Cannot be resized by accident.
    ncols: usize,
}

impl BoolTable {
    pub fn new(nrows: usize, ncols: usize) -> Self {
        BoolTable {
            data: vec![false; nrows * ncols].into_boxed_slice(),
            ncols,
        }
    }

    pub fn row(&self, r: usize) -> &[bool] {
        let start = r * self.ncols;
        &self.data[start..(start + self.ncols)]
    }

    pub fn row_mut(&mut self, r: usize) -> &mut [bool] {
        let start = r * self.ncols;
        &mut self.data[start..(start + self.ncols)]
    }
}

pub(crate) struct DenseTagTable {
    flags: BoolTable,
    files: Box<[String]>,
    tags: Box<[String]>,
    tag_indices: HashMap<String, usize>,
}

impl DenseTagTable {
    pub fn from_dir(dirpath: PathBuf) -> Result<DenseTagTable, FstoreError> {
        let TagTable {
            root: _root,
            index_map: tag_indices,
            table: sparse,
        } = TagTable::from_dir(dirpath)?;
        let tags: Box<[String]> = {
            let mut pairs: Vec<_> = tag_indices.iter().collect();
            pairs.sort_by(|(_t1, i1), (_t2, i2)| i1.cmp(i2));
            pairs.into_iter().map(|(t, _i)| t.clone()).collect()
        };
        let (files, flags) = {
            let mut pairs: Vec<_> = sparse
                .into_iter()
                .map(|(p1, f1)| (format!("{}", p1.display()), f1))
                .collect();
            pairs.sort_by(|(path1, _flags1), (path2, _flags2)| path1.cmp(path2));
            let (files, flags): (Vec<_>, Vec<_>) = pairs.into_iter().unzip();
            let mut dense = BoolTable::new(files.len(), tags.len());
            for (src, i) in flags.iter().zip(0..flags.len()) {
                debug_assert!(tags.len() >= src.len());
                let dst = dense.row_mut(i);
                let dst = &mut dst[..src.len()];
                dst.copy_from_slice(src); // Requires the src and dst to be of same length.
            }
            (files.into_boxed_slice(), dense)
        };
        Ok(DenseTagTable {
            flags,
            files,
            tags,
            tag_indices,
        })
    }

    pub fn flags(&self, row: usize) -> &[bool] {
        return self.flags.row(row);
    }

    pub fn tags(&self) -> &[String] {
        &self.tags
    }

    pub fn files(&self) -> &[String] {
        &self.files
    }
}
