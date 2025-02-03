use crate::{
    core::Error,
    filter::Filter,
    load::{
        get_filename_str, infer_implicit_tags, FileLoadingOptions, GlobMatches, LoaderOptions, Tag,
    },
    walk::{DirTree, MetaData, VisitedDir},
};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
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

/// Returns the number of files and the number of tags.
pub fn count_files_tags(path: PathBuf) -> Result<(usize, usize), Error> {
    let mut matcher = GlobMatches::new();
    let mut alltags = HashSet::new();
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
                alltags.extend(data.alltags.iter().map(|t| t.to_string()).chain(
                    infer_implicit_tags(get_filename_str(rel_dir_path)?).map(|t| t.to_string()),
                ));
                // Collect all tracked files.
                matcher.find_matches(files, &data.globs, false);
                files.iter().enumerate().fold(0usize, |numfiles, (fi, f)| {
                    match matcher.is_file_matched(fi) {
                        true => {
                            if let Some(name) = f.name().to_str() {
                                alltags.extend(infer_implicit_tags(name).map(|t| t.to_string()));
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
    let filter = Filter::parse(filter, |tag| {
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
    let mut filetags = vec![false; tag_index.len()].into_boxed_slice();
    while let Some(VisitedDir {
        traverse_depth,
        rel_dir_path,
        files,
        metadata,
        ..
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
                .map(|t| Tag::Text(t))
                .chain(infer_implicit_tags(get_filename_str(rel_dir_path)?))
                .filter_map(|tag| match tag {
                    Tag::Text(t) | Tag::Format(t) => tag_index.get(t).copied(),
                    Tag::Year(y) => tag_index.get(&y.to_string()).copied(),
                }),
        );
        // Process all files in the directory.
        matcher.find_matches(files, &data.globs, false);
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
                        .map(|t| Tag::Text(t))
                })
                // Implicit tags.
                .chain(infer_implicit_tags(
                    file.name()
                        .to_str()
                        .ok_or(Error::InvalidPath(file.name().into()))?,
                ))
                .filter_map(|tag| match tag {
                    Tag::Text(t) | Tag::Format(t) => tag_index.get(t).copied(),
                    Tag::Year(y) => tag_index.get(&y.to_string()).copied(),
                })
                .chain(inherited.tag_indices.iter().copied())
            {
                filetags[index] = true;
            }
            if filter.eval(|ti| filetags[ti]) {
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
    data: Box<[bool]>, // Boxed, so that it cannot be resized by accident.
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
}

/// This is similar to a `TagTable`, but the flags indicating in which
/// file has which tags are stored in a dense 2d array rather than a
/// sparse hash-map of vectors.
pub struct TagTable {
    root: PathBuf,
    flags: BoolTable,
    files: Box<[String]>,
    tags: Box<[String]>,
    tag_index: HashMap<String, usize>,
}

impl TagTable {
    fn get_tag_index(tag: String, map: &mut HashMap<String, usize>) -> usize {
        let size = map.len();
        *(map.entry(tag).or_insert(size))
    }

    pub fn from_dir(dirpath: PathBuf) -> Result<TagTable, Error> {
        let mut tag_index = HashMap::new();
        let mut allfiles = Vec::new();
        let mut table = HashSet::<(usize, usize)>::new();
        let mut inherited = InheritedTags {
            tag_indices: Vec::new(),
            offsets: Vec::new(),
            depth: 0,
        };
        let mut matcher = GlobMatches::new();
        let mut filetags: Vec<String> = Vec::new();
        let mut dir = DirTree::new(
            dirpath.clone(),
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
            rel_dir_path,
            files: dirfiles,
            metadata,
            ..
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
                    .map(|t| Tag::Text(t))
                    .chain(infer_implicit_tags(get_filename_str(rel_dir_path)?))
                    .map(|tag| match tag {
                        Tag::Text(t) | Tag::Format(t) => {
                            Self::get_tag_index(t.to_string(), &mut tag_index)
                        }
                        Tag::Year(y) => Self::get_tag_index(y.to_string(), &mut tag_index),
                    }),
            );
            // Process all files in the directory.
            matcher.find_matches(dirfiles, &data.globs, false);
            allfiles.reserve(dirfiles.len());
            for (fi, file) in dirfiles
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
                        .chain(
                            infer_implicit_tags(
                                file.name()
                                    .to_str()
                                    .ok_or(Error::InvalidPath(file.name().into()))?,
                            )
                            .map(|t| t.to_string()),
                        ),
                );
                let file_index = allfiles.len();
                allfiles.push(format!(
                    "{}",
                    {
                        let mut relpath = rel_dir_path.to_path_buf();
                        relpath.push(file.name());
                        relpath
                    }
                    .display()
                ));
                table.extend(
                    filetags
                        .drain(..)
                        .map(|tag| (file_index, Self::get_tag_index(tag, &mut tag_index))) // This file's explicit tags.
                        .chain(inherited.tag_indices.iter().map(|ti| (file_index, *ti))), // Inherited tags.
                );
            }
        }
        // Construct the bool-table.
        let ntags = tag_index.len();
        let mut flags = BoolTable::new(allfiles.len(), ntags);
        for i in table.into_iter().map(move |(fi, ti)| fi * ntags + ti) {
            flags.data[i] = true;
        }
        Ok(TagTable {
            root: dirpath,
            flags,
            files: allfiles.into_boxed_slice(),
            tags: {
                // Vec of tags sorted by their indices.
                let mut pairs: Vec<_> = tag_index.iter().collect();
                pairs.sort_unstable_by(|(_t1, i1), (_t2, i2)| i1.cmp(i2));
                pairs.into_iter().map(|(t, _i)| t.clone()).collect()
            },
            tag_index,
        })
    }

    pub fn path(&self) -> &Path {
        &self.root
    }

    pub fn flags(&self, file: usize) -> &[bool] {
        self.flags.row(file)
    }

    pub fn tags(&self) -> &[String] {
        &self.tags
    }

    pub fn files(&self) -> &[String] {
        &self.files
    }

    pub fn tag_parse_fn(&self) -> impl Fn(&str) -> Filter + use<'_> {
        |tag| match self.tag_index.get(tag) {
            Some(i) => Filter::Tag(*i),
            None => Filter::FalseTag,
        }
    }
}
