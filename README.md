# ftag

ftag is a tool that lets you attach tags and descriptions to files, then later
query and retrieve files based on their tags. This is designed for use with
large archives of files such as documents or pictures.

## Motivation

Directory trees are not a good solution for organizing large archives of
files. There is never a clear logic behind why one set of directories should be
higher in the directory tree than another set of directories. You can easily end
up with deep directory trees, which are difficult to navigate and make it
difficult to retrieve a file that you are looking for. `ftag` is written to
solve exactly this problem. Associating arbitrary tags with files, is a much
more flexible and powerful way of organizing files. This allows you to search
for files using any combination of tags. This also lets you have a relatively
flat directory structure and make it easy to find the files you're looking for.

While there are other tools providing similar features, some are built in to
operating systems, `ftag` is designed to be simple, reliable and suitable for
long term archiving. All your tags and metadata are stored in plain text
files. If you move or copy a directory, the plain text file(s) containing the
metadata for that directory and all the files within get moved or copied with
it. Because ftag uses plain text files, it is perfect for long term archives, as
it gives you full ownership of your data. And obviously, metadata is platform
agnostic.

## Installation

Ftag is written in Rust and is available on
[crates.io](https://crates.io/):
```bash
cargo install ftag
```

Or you can install from this repository:

```bash
cargo install --git https://github.com/ranjeethmahankali/ftag.git
```

## Usage

`.ftag` files contain all the metadata, i.e. tags and descriptions for your
files and directories. You can place a `.ftag` file anywhere in your file
system, in whichever directory you wish to organize using tags. Each `.ftag`
file should contain the metadata for the directory containing that `.ftag` file
and the files that are immediate children of that directory, i.e. siblings of
the `.ftag` file. It should not contain the metadata for any of the nested
directories. The nested directories should use their own `.ftag` files. It is
important to keep the metadata decentralized in this way, so that when you move
or copy the directories, you don't invalidate the metadata.

By design, `ftag` never modifies the `.ftag` files. The only exception to this
principle is the `clean` command, but even that just reorganizes the data
authored by the user and makes a backup of the original file. Generally, `.ftag`
files are meant to be authored by the user, and only consumed and queried by
`ftag`. As an Emacs user myself, I wrote [this major
mode](https://github.com/ranjeethmahankali/ftag-mode) which provides
autocompletion, file preview etc. and makes authoring `.ftag` files a breeze (I
haven't written plugins for any other editor but if you like `ftag`, feel free
to contribute!). Further details of `.ftag` files are discussed later.

### `ftag` CLI tool

This query will recursively traverse the directory structure from the working
directory and print a list of files that are tagged with "my-tag".

```bash
ftag query my-tag
```

You can get fancy with your queries and compose Boolean expressions using
tags. The symbols used for composing Boolean expressions are: `&` for AND, `|`
for OR, `!` for NOT, and `()` for nesting expressions. `-q` is an alias for the
`query` command. This command will traverse the directories and output a list of
files that satisfy the provided query string. This includes files that have
"my-tag" and "other-tag" and do not have "exclude-tag" OR have both "tag1" and
"tag2".

```bash
ftag -q 'my-tag & other-tag & !exclude-tag | (tag1 & tag2)'
```

A slightly simpler version of a query is called "search". This command takes a
search string, breaks it up into keywords and finds all files that contain any
of these keywords either among their tags or within their description. The
comparison is case insensitive.

```bash
ftag search 'string of my keywords'
# OR
ftag -s 'string of my keywords'
```

Below command will traverse the directories and check to make sure all `.ftag`
files are valid, i.e. the metadata contained within them has not been
invalidated due to a renaming, moving or deleting files.

```bash
ftag check
```

Below command will produce a list of tags for the given directory or file, and a
description. The description is just a string that was authored by the user to
describe the file.

```bash
ftag whatis path/to/my/file
```

If you wish to modify the metadata, the `edit` command opens up the `.ftag` in
the given directory in your default editor. If no directory is provided, the
current working directory is assumed.

```bash
ftag edit path/to/directory
# OR
ftag edit # Edit working directory
```

When you start tagging a large collection of existing files, you won't be able
to author the metadata for all of them in one sitting. It is often useful to see
a list of files that are not tracked, i.e. are not assigned any metadata. This
command will produce a list of untracked files.

```bash
ftag untracked
```

Below command will traverse the directories recursively and produce a list of all
tags. As this command walks the directories recursively, if a directory doesn't
contain a `.ftag` file, it is ignored. It is assumed that you don't wish to
track the files in that directory and they are not reported as untracked. The
tags are sorted lexicographically.

```bash
ftag tags
```

If you want to know the number of files tracked by ftag, and the total number of
tags associated with said files, from your current working directory
recursively, use this command:

```bash
ftag count
```

Over time, the data stored in the `.ftag` files can go out of sync from the
actual files on the disk, due to moving, renaming, or deleting files. The
`clean` command removes any file paths / globs that don't match files present on
disk, and also merges entries that share the same tags and description into a
single entry. As a safety measure, this command also creates a backup file
`.ftagbak` before making changes to the `.ftag` file.

```bash
ftag clean
```

Most `ftag` subcommands recursively traverse the directory from the current
working directory and produce the output that you asked for. If you wish to
produce to same output from a different path instead of the current working
directory, you can override it by providing a `--path | -p` flag.

```bash
ftag --path different/starting/directory <COMMAND>
```

### Bash Autocompletion

When searching for files, you may not remember the exact tags you're
supposed to search for. Having autocompletion for tags and commands
can be very helpful. To enable tab-autocompletion in bash, add this to your bash profile:
```bash
complete -o default -C 'ftag --bash-complete' ftag
```

### Interactive mode with TUI

If you really don't know what tags to query, interactive mode can be very
helpful. It recursively traverses all directories, loads all metadata and starts
an interactive session with a TUI. You'll see a list of all tracked files, and a
union of all tags these files have. You can figure out what to search for by
looking through the list of tags, or start typing something like `tag1 & tag..`
and let the autocompletion help you find the right tag. If you hit return, the
list of files is filtered down to only the ones that satisfy the
query. Similarly you should also see the collection of tags shrink. You can
iteratively refine your search until you find the file you are looking for. If
you simply start typing text into the REPL interface, the text is interpreted as
a filter string. If you begin with a `/`, then the text is interpreted as a
command.

Commands you can use in interactive mode are:
- `/reset` to remove the current filter
- `/whatis <index>` to see the tags and description of the file in the current
  list. You choose the file by it's index rather than name or path.
- `/open <index>` to open the file with the given index in your default
  application.
- `/quit` or `exit` will exit out of the interactive mode.

If you already have a filter applied, and are looking at a narrowed down list of
files, the next filter you type will be appended to the existing filter. For
example, if you enter the filter `tag1 & tag2`, and hit return, that filter is
applied and a smaller list of files is shown. If you then type `| tag3` and hit
return, it will be appended to the existing filter resulting in `(tag1 & tag2) |
tag3`. This is useful when incrementally tightening the filter to find the file
you want. At any point, pressing the up and down arrow keys lets you scroll
through the list of active tags.

### `.ftag` Files

The format of a `.ftag` file should be a header, followed by content under that
header, followed by another header and so on till the end of the file. Supported
headers are: `desc` for description, `tags` for tags and `path` for filepaths
and globs. Headers should be in their own line, wrapped in `[]` brackets,
similar to TOML or INI files. So a typical `.ftag` file might look like:

```ini
[desc]
This is the description of this directory.
The description can span multiple lines and paragraphs.

[tags]
tag1 tag2 tag3 tag4
tag5
tag6

[path]
my_file_1.pdf
[tags]
ftag1 pdf document

[path]
my_file_2.pdf
[desc]
This is my second file.
[tags]
ftag2 pdf document

[path]
my_file_3.pdf
my_file_4.pdf
my_file_5.pdf
[desc]
These are the rest of my files in this directory. This description will apply to
all of them at once. The tags below will also apply to all these files at
once. If these files appared once before in the file, this description and below
tags will be concatenated to the previously declared description and tags.
[tags]
pdf document miscellaneous
```

Tags and description headers that occur at the start of the `.ftag` file are
associated with the directory itself. Tags and description headers that occur
after a `path` header are associated with that specific file or glob. `path`
doesn't need to be one specific file. Instead it can be a glob, in which case,
the provided tags and description are applied to all files that match the
glob. Globs can be used to avoid repetition when you want to associate the same
set of tags with many files. As such, multiple globs can match a single
file. The tags associated with that file will be the union of tags associated
with the globs that match the file. The descriptions are concatenated. The
`path` header can also contain more than one file / glob. The following tags and
description will be applied to all the files listed or files that match any of
the globs listed. When the `path` header contains multiple files / globs, they
must be listed one per line.

### `ftagui`: GUI application

Interactive mode is useful for incrementally refining the filtering criteria and
searching through a collection of files. But it is not very convenient for
photos. For documents, it might be enough to see the file path in the
interactive mode. But often photos don't have descriptive names, and it is not
convenient to have to give photos descriptive names. It is much more convenient
to just look at the photos as you search through them. `ftagui` is a GUI
application designed to solve this problem. The UI layout and the interactions
in `ftagui` are designed to be very similar to the interactive TUI mode of
`ftag`, with a few small differences.

- You can scroll through the list of tags on the left side.
- You'll see the previews of all the files in the center of the screen.
- The primary mode of interaction is through a text field at the bottom in which
  you can enter your commands. These commands are the same as those in the
  interactive mode.
- The file previews are paginated.
- The current query filter and the page number are shown at the top.
- You can press `Ctrl + n` to go to the next page, and `Ctrl + p` to go to the
  previous page.
- Hovering the mouse on an image will show you the tags and the description of
  the image in a tooltip.
- Double clicking the image will open it in the default application.

## Performance and Memory Usage

A lot of care and thought went into making `ftag` fast and efficient, and is
constantly being improved in this regard. Everything is designed to run on a
single thread, and run as fast as possible so as not to require threads or
asynchronous tasks. For example, if you're using the CLI you can expect various
commands to run in a few milliseconds for archives managing tens of thousands of
files. Starting a TUI interactive session may take tens of milliseconds for an
archive of that size.

A lot of care also went into minimizing the memory usage. Wherever possible,
`ftag` will avoid allocating large data structures, and instead walk the
directory tree performing various tasks on the files / directories as it
goes. So if you run a query on a large directory tree with an extraordinary
total number of files, the resource usage should only be as large as required by
the largest single directory in the directory tree. Obviously this is not true
for the commands that launch an interactive session (TUI and GUI), because they
have to load all the metadata of the entire archive into memory to start the
session. But this is true for all the CLI commands wherever possible.
