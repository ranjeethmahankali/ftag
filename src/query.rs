use crate::{
    core::Error,
    filter::Filter,
    load::{get_filename_str, implicit_tags_str, FileLoadingOptions, GlobMatches, LoaderOptions},
    walk::{DirTree, MetaData, VisitedDir},
};
use ahash::{AHashMap, AHashSet};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

/*
When a tags are specified for a folder, it's subfolders and all their subfolders
inherit those tags. This inheritance follows the directory tree. When
recursively loading all the data starting from a root directory, we have to keep
track of inherited tags. We do this by pushing all the tags into a vector, and
by storing the offsets that separate contiguous chunks of this vector across the
depth-first chain of directories currently being traversed.
 */
struct InheritedTags {
    /// Indices of currently loaded tags.
    tag_indices: Vec<usize>,
    /// Offsets that separate the tags across the depth-first chain of directories currently being traversed.
    offsets: Vec<usize>,
    /// Current depth of the traversal.
    depth: usize,
}

impl InheritedTags {
    /// Update the inherited tags for the specified `newdepth`. A new depth that
    /// is 1 more than the current depth implies traversing deeper into the
    /// directory tree. A new depth that is smaller than the current depth
    /// implies popping all the tags inherited from folders deeper than the new
    /// depth.
    fn update(&mut self, newdepth: usize) -> Result<(), Error> {
        if self.depth + 1 == newdepth {
            self.offsets.push(self.tag_indices.len());
        } else if self.depth >= newdepth {
            let mut marker = self.tag_indices.len();
            for _ in 0..(self.depth + 1 - newdepth) {
                marker = self.offsets.pop().ok_or(Error::DirectoryTraversalFailed)?;
            }
            self.tag_indices.truncate(marker);
            self.offsets.push(marker);
        } else {
            return Err(Error::DirectoryTraversalFailed);
        }
        self.depth = newdepth;
        Ok(())
    }
}

/// Table of all files and tags, that can be loaded recursively from any path.
pub(crate) struct TagTable {
    root: PathBuf,
    tag_index: AHashMap<String, usize>,
    files: Vec<PathBuf>,
    table: AHashSet<(usize, usize)>,
}

impl TagTable {
    /// Create a new tag table by recursively traversing directories
    /// from `dirpath`.
    pub(crate) fn from_dir(dirpath: PathBuf) -> Result<Self, Error> {
        let mut table = TagTable {
            root: dirpath,
            tag_index: AHashMap::new(),
            files: Vec::new(),
            table: AHashSet::new(),
        };
        let mut inherited = InheritedTags {
            tag_indices: Vec::new(),
            offsets: Vec::new(),
            depth: 0,
        };
        let mut matcher = GlobMatches::new();
        let mut filetags: Vec<String> = Vec::new();
        let mut dir = DirTree::new(
            table.root.clone(),
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
            traverse_depth,
            abs_dir_path,
            rel_dir_path,
            files,
            metadata,
        }) = dir.walk()
        {
            inherited.update(traverse_depth)?;
            let data = match metadata {
                MetaData::Ok(d) => d,
                MetaData::NotFound => continue,
                MetaData::FailedToLoad(e) => return Err(e),
            };
            // Push directory tags.
            inherited.tag_indices.extend(
                data.tags()
                    .iter()
                    .map(|t| t.to_string())
                    .chain(implicit_tags_str(get_filename_str(abs_dir_path)?))
                    .map(|tag| Self::get_tag_index(tag, &mut table.tag_index)),
            );
            // Process all files in the directory.
            matcher.find_matches(files, &data.globs, false);
            table.files.reserve(files.len());
            for (fi, file) in files
                .iter()
                .enumerate()
                // Only interested in tracked files.
                .filter(|(fi, _)| matcher.is_file_matched(*fi))
            {
                filetags.clear();
                filetags.extend(
                    matcher
                        .matched_globs(fi) // Tags associated with matching globs.
                        .flat_map(|gi| {
                            data.globs[gi]
                                .tags(&data.alltags)
                                .iter()
                                .map(|t| t.to_string())
                        })
                        // Implicit tags.
                        .chain(implicit_tags_str(
                            file.name()
                                .to_str()
                                .ok_or(Error::InvalidPath(file.name().into()))?,
                        )),
                );
                table.add_file(
                    {
                        let mut relpath = rel_dir_path.to_path_buf();
                        relpath.push(file.name());
                        relpath
                    },
                    &mut filetags,
                    &inherited.tag_indices,
                );
            }
        }
        Ok(table)
    }

    fn get_tag_index(tag: String, map: &mut AHashMap<String, usize>) -> usize {
        let size = map.len();
        *(map.entry(tag).or_insert(size))
    }

    fn add_file(&mut self, path: PathBuf, tags: &mut Vec<String>, inherited: &[usize]) {
        let fi = self.files.len();
        self.files.push(path);
        self.table.extend(
            tags.drain(..)
                .map(|tag| (fi, Self::get_tag_index(tag, &mut self.tag_index))) // This file's explicit tags.
                .chain(inherited.iter().map(|ti| (fi, *ti))), // Inherited tags.
        );
    }
}

/// Returns the number of files and the number of tags.
pub fn count_files_tags(path: PathBuf) -> Result<(usize, usize), Error> {
    let mut matcher = GlobMatches::new();
    let mut alltags = AHashSet::new();
    let mut numfiles = 0usize;
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
        files,
        metadata,
        ..
    }) = dir.walk()
    {
        match metadata {
            MetaData::FailedToLoad(e) => return Err(e),
            MetaData::NotFound => continue,
            MetaData::Ok(data) => {
                // Collect all tags.
                alltags.extend(
                    data.alltags
                        .iter()
                        .map(|t| t.to_string())
                        .chain(implicit_tags_str(get_filename_str(rel_dir_path)?)),
                );
                // Collect all tracked files.
                matcher.find_matches(files, &data.globs, false);
                files.iter().enumerate().fold(0usize, |numfiles, (fi, f)| {
                    match matcher.is_file_matched(fi) {
                        true => {
                            if let Some(name) = f.name().to_str() {
                                alltags.extend(implicit_tags_str(name));
                            }
                            numfiles + 1
                        }
                        false => numfiles,
                    }
                });
                numfiles += files
                    .iter()
                    .enumerate()
                    .filter(|(fi, _file)| matcher.is_file_matched(*fi))
                    .count();
            }
        }
    }
    Ok((numfiles, alltags.len()))
}

pub fn run_query(dirpath: PathBuf, filter: &str) -> Result<(), Error> {
    let mut tag_index = BTreeMap::<String, usize>::new();
    let filter = Filter::<usize>::parse(filter, |tag| {
        let size = tag_index.len();
        let index = *tag_index.entry(tag.to_string()).or_insert(size);
        Filter::Tag(index)
    })
    .map_err(Error::InvalidFilter)?;
    let tag_index = tag_index; // Immutable.
    let mut inherited = InheritedTags {
        tag_indices: Vec::new(),
        offsets: Vec::new(),
        depth: 0,
    };
    let mut matcher = GlobMatches::new();
    let mut dir = DirTree::new(
        dirpath,
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
        traverse_depth,
        abs_dir_path,
        rel_dir_path,
        files,
        metadata,
    }) = dir.walk()
    {
        inherited.update(traverse_depth)?;
        let data = match metadata {
            MetaData::Ok(d) => d,
            MetaData::NotFound => continue,
            MetaData::FailedToLoad(e) => return Err(e),
        };
        // Push directory tags.
        inherited.tag_indices.extend(
            data.tags()
                .iter()
                .map(|t| t.to_string())
                .chain(implicit_tags_str(get_filename_str(abs_dir_path)?))
                .filter_map(|tag| tag_index.get(&tag).copied()),
        );
        // Process all files in the directory.
        matcher.find_matches(files, &data.globs, false);
        let mut filetags = vec![false; tag_index.len()].into_boxed_slice(); // TODO: Consider using a bitset or a bit vec.
        for (fi, file) in files
            .iter()
            .enumerate()
            .filter(|(fi, _)| matcher.is_file_matched(*fi))
        {
            filetags.fill(false);
            for index in matcher
                .matched_globs(fi) // Tags associated with matching globs.
                .flat_map(|gi| {
                    data.globs[gi]
                        .tags(&data.alltags)
                        .iter()
                        .map(|t| t.to_string())
                })
                // Implicit tags.
                .chain(implicit_tags_str(
                    file.name()
                        .to_str()
                        .ok_or(Error::InvalidPath(file.name().into()))?,
                ))
                .filter_map(|tag| tag_index.get(&tag).copied())
                .chain(inherited.tag_indices.iter().copied())
            {
                filetags[index] = true;
            }
            if filter.eval(&|ti| filetags[ti]) {
                let mut path = rel_dir_path.to_path_buf();
                path.push(file.name());
                println!("{}", path.display());
            }
        }
    }
    Ok(())
}

/// 2d array of bools.
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

/// This is similar to a `TagTable`, but the flags indicating in which
/// file has which tags are stored in a dense 2d array rather than a
/// sparse hash-map of vectors.
pub struct DenseTagTable {
    root: PathBuf,
    flags: BoolTable,
    files: Box<[String]>,
    tags: Box<[String]>,
    tag_index: AHashMap<String, usize>,
}

impl DenseTagTable {
    pub fn from_dir(dirpath: PathBuf) -> Result<DenseTagTable, Error> {
        let TagTable {
            root,
            tag_index,
            mut files,
            table,
        } = TagTable::from_dir(dirpath)?;
        let tags: Box<[String]> = {
            // Vec of tags sorted by their indices.
            let mut pairs: Vec<_> = tag_index.iter().collect();
            pairs.sort_unstable_by(|(_t1, i1), (_t2, i2)| i1.cmp(i2));
            pairs.into_iter().map(|(t, _i)| t.clone()).collect()
        };
        let files: Box<[String]> = files
            .drain(..)
            .map(|f| format!("{}", f.display()))
            .collect();
        // Construct the bool-table.
        let mut flags = BoolTable::new(files.len(), tags.len());
        for (fi, ti) in table.into_iter() {
            flags.row_mut(fi)[ti] = true;
        }
        Ok(DenseTagTable {
            root,
            flags,
            files,
            tags,
            tag_index,
        })
    }

    pub fn path(&self) -> &Path {
        &self.root
    }

    pub fn flags(&self, row: usize) -> &[bool] {
        self.flags.row(row)
    }

    pub fn tags(&self) -> &[String] {
        &self.tags
    }

    pub fn files(&self) -> &[String] {
        &self.files
    }

    pub fn tag_index(&self) -> &AHashMap<String, usize> {
        &self.tag_index
    }
}
