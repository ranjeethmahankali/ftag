use crate::{
    core::{get_relative_path, Error},
    filter::{Filter, TagMaker},
    load::{
        get_store_path, implicit_tags_os_str, DirData, FileLoadingOptions, GlobMatches, Loader,
        LoaderOptions,
    },
    walk::WalkDirectories,
};
use std::collections::HashMap;
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

struct InheritedTags {
    tag_indices: Vec<usize>,
    offsets: Vec<usize>,
    depth: usize,
}

impl InheritedTags {
    fn update(&mut self, newdepth: usize) -> Result<(), Error> {
        if self.depth + 1 == newdepth {
            self.offsets.push(self.tag_indices.len());
        } else if self.depth >= newdepth {
            let mut marker = self.tag_indices.len();
            for _ in 0..(self.depth + 1 - newdepth) {
                marker = self
                    .offsets
                    .pop()
                    .ok_or(Error::TagInheritanceFailed)?;
            }
            self.tag_indices.truncate(marker);
            self.offsets.push(marker);
        } else {
            return Err(Error::DirectoryTraversalFailed);
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
    fn query<'a>(&'a self, filter: Filter<usize>) -> impl Iterator<Item = std::path::Display<'a>> {
        self.table.iter().filter_map(move |(path, flags)| {
            if filter.eval_vec(flags) {
                Some(path.display())
            } else {
                None
            }
        })
    }

    pub(crate) fn from_dir(dirpath: PathBuf) -> Result<Self, Error> {
        // These structs are locally defined because their only
        // purpose is to use with serde_yaml to extract relevant
        // information from the YAML files.
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
        let mut gmatcher = GlobMatches::new();
        let mut filetags: Vec<String> = Vec::new();
        let mut loader = Loader::new(LoaderOptions::new(
            true,
            false,
            FileLoadingOptions::Load {
                file_tags: true,
                file_desc: false,
            },
        ));
        while let Some((depth, curpath, children)) = walker.next() {
            inherited.update(depth)?;
            // Deserialize yaml without copy.
            let DirData {
                tags,
                files,
                desc: _,
            } = {
                match get_store_path::<true>(&curpath) {
                    Some(path) => loader.load(&path)?,
                    None => continue,
                }
            };
            // Push directory tags.
            for tag in tags
                .iter()
                .map(|t| t.to_string())
                .chain(implicit_tags_os_str(curpath.file_name()))
            {
                inherited.tag_indices.push(Self::get_tag_index(
                    tag,
                    &mut table.index_map,
                    &mut num_tags,
                ));
            }
            // Process all files in the directory.
            gmatcher.find_matches(children, &files, false);
            for (ci, cpath) in children
                .iter()
                .filter_map(|ch| get_relative_path(&curpath, ch.name(), &rootdir))
                .enumerate()
            {
                filetags.clear();
                let mut found: bool = false;
                for fi in gmatcher.matched_globs(ci) {
                    found = true;
                    filetags.extend(files[fi].tags.iter().map(|t| t.to_string()));
                    filetags.extend(implicit_tags_os_str(cpath.file_name()));
                }
                if found {
                    table.add_file(cpath, &mut filetags, &mut num_tags, &inherited.tag_indices);
                }
            }
        }
        return Ok(table);
    }

    fn get_tag_index(tag: String, map: &mut HashMap<String, usize>, counter: &mut usize) -> usize {
        let size = map.len();
        let entry = *(map.entry(tag.to_string()).or_insert(size));
        *counter = map.len();
        return entry;
    }

    fn add_file(
        &mut self,
        path: PathBuf,
        tags: &mut Vec<String>,
        num_tags: &mut usize,
        inherited: &Vec<usize>,
    ) {
        let flags = self.table.entry(path).or_insert(Vec::new());
        // Set the file's explicit tags.
        flags.reserve(flags.len() + tags.len());
        for tag in tags.drain(..) {
            safe_set_flag(
                flags,
                Self::get_tag_index(tag, &mut self.index_map, num_tags),
            );
        }
        // Set inherited tags.
        for i in inherited {
            safe_set_flag(flags, *i);
        }
    }
}

impl TagMaker<usize> for TagTable {
    fn create_tag(&self, input: &str) -> Filter<usize> {
        match self.index_map.get(&input.to_string()) {
            Some(i) => Filter::Tag(*i),
            None => Filter::FalseTag,
        }
    }
}

pub(crate) fn run_query(dirpath: PathBuf, filter: &String) -> Result<(), Error> {
    let table = TagTable::from_dir(dirpath)?;
    let filter = Filter::<usize>::parse(filter.as_str(), &table)
        .map_err(|e| Error::InvalidFilter(e))?;
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
    root: PathBuf,
    flags: BoolTable,
    files: Box<[String]>,
    tags: Box<[String]>,
    tag_indices: HashMap<String, usize>,
}

impl DenseTagTable {
    pub fn from_dir(dirpath: PathBuf) -> Result<DenseTagTable, Error> {
        let TagTable {
            root,
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
            root,
            flags,
            files,
            tags,
            tag_indices,
        })
    }

    pub fn path(&self) -> &Path {
        &self.root
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

impl TagMaker<usize> for DenseTagTable {
    fn create_tag(&self, input: &str) -> Filter<usize> {
        match self.tag_indices.get(&input.to_string()) {
            Some(i) => Filter::Tag(*i),
            None => Filter::FalseTag,
        }
    }
}
