# fstore

fstore is a tool that lets you attach tags and descriptions to files,
then later query and retrieve files based on their tags. This is
designed for use with large archives of files such as documents or
pictures.

## Motivation

Directory trees are not a good solution for organizing large archives
of files. There is never a clear logic behind why one set of
directories should be higher in the directory tree than another set of
directories. You can easily end up with deep directory trees, which
are difficult to navigate and make it difficult to retrieve a file
that you are looking for. `fstore` is written to solve exactly this
problem. Associating arbitrary tags with files, is a much more
flexible and powerful way of organizing files. This allows you to
search for files using any combination of tags. This also lets you
have a relatively flat directory structure and make it easy to find
the files you're looking for.

While there are other tools providing similar features, `fstore` is
designed to be simple, reliable and suitable for long term
archiving. All your tags and metadata are stored in plain text
files. If you move or copy a directory, the plain text file(s)
containing the metadata for that directory and all the files within
get moved or copied with it. Because fstore uses plain text files, it
is perfect for long term archives, as it gives full ownership of your
own data to you.

## Installation

Fstore is written in Rust. It is not available as a crate yet, but you
can clone this repo install it using cargo by running:

```bash
cargo install --path /path/to/fstore/repo/
```

## Usage

`.fstore` files contain all the metadata, i.e. tags and descriptions
for your files and directories. You can place a `.fstore` file
anywhere in your file system, in whichever directory you wish to
organize using tags. Each `.fstore` file should contain the metadata
for the directory containing that `.fstore` file and the files that
are immediate children of that directory, i.e. siblings of the
`.fstore` file. It should not contain the metadata for any of the
nested directories. The nested directories should use their own
`.fstore` files. It is important to keep the metadata decentralized in
this way, so that when you move or copy the directories, you don't
invalidate the metadata.

By design, `fstore` never modifies the `.fstore` files. `.fstore`
files are meant to be authored by the user, and only consumed and
queried by `fstore`. As an Emacs user myself, I wrote [this major
mode](https://github.com/ranjeethmahankali/fstore-mode) which provides
autocompletion, file preview etc. and makes authoring `.fstore` files
a breeze (I haven't written plugins for any other editor but if you
like `fstore`, feel free to contribute!). Further details of `.fstore`
files are discussed later.

### `fstore` CLI tool

This query will recursively traverse the directory structure from the
working directory and print a list of files that are tagged with
"my-tag".

```bash
fstore query my-tag
```

You can get fancy with your queries and compose Boolean expressions
using tags. The symbols used for composing Boolean expressions are:
`&` for AND, `|` for OR, `!` for NOT, and `()` for nesting
expressions. `-q` is an alias for the `query` command. This command
will traverse the directories and output a list of files that satisfy
the provided query string. This includes files that have "my-tag" and
"other-tag" and do not have "exclude-tag" OR have both "tag1" and
"tag2".

```bash
fstore -q 'my-tag & other-tag & !exclude-tag | (tag1 & tag2)'
```

Below command will traverse the directories and check to make sure all
`.fstore` files are valid, i.e. the metadata contained within them has
not been invalidated due to a renaming, moving or deleting files.

```bash
fstore check
```

Below command will produce a list of tags for the given directory or
file, and a description. The description is just a string that was
authored by the user to describe the file.

```bash
fstore whatis path/to/my/file
```

If you wish to modify the metadata, the `edit` command opens up the
`.fstore` in the given directory in your default editor. If no
directory is provided, the current working directory is assumed.

```bash
ftore edit path/to/directory
# OR
fstore edit # Edit working directory
```

When you start tagging a large collection of existing files, you won't
be able to author the metadata for all of them in one sitting. It is
often useful to see a list of files that are not tracked, i.e. are not
assigned any metadata. This command will produce a list of untracked
files.

```bash
fstore untracked
```

This command will traverse the directories recursively and produce a
list of all tags. As this command walks the directories recursively,
if a directory doesn't contain a `.fstore` file, it is ignored. It is
assumed that you don't wish to track the files in that directory and
they are not reported as untracked.

```bash
fstore tags
```

Most `fstore` subcommands recursively traverse the directory from the
current working directory and produce the output that you asked
for. If you wish to produce to same output from a different path
instead of the current working directory, you can override it by
providing a `--path | -p` flag.

```bash
fstore --path different/starting/directory
```

### Bash autocompletion

When searching for files, you may not remember the exact tags you're
supposed to search for. Having autocompletion for tags and commands
can be very helpful. To enable tab-autocompletion in bash, add this to your bash profile:
```bash
complete -o default -C 'fstore --bash-complete' fstore
```

### Interactive mode with TUI

If you really don't know what tags to query, interactive mode can be
very helpful. It recursively traverses all directories, loads all
metadata and starts an interactive session with a TUI. You'll see a
list of all tracked files, and a union of all tags these files
have. You can figure out what to search for by looking through the
list of tags, or start typing something like `filter tag1 & tag..` and
let the autocompletion help you find the right tag. If you hit return,
the list of files is filtered down to only the ones that satisfy the
query. Similarly you should also see the collection of tags
shrink. You can iteratively refine your search until you find the file
you are looking for.

Other commands you can use in interactive mode are:
- `reset` to remove the current filter
- `whatis <index>` to see the tags and description of the file in the
  current list. You choose the file by it's index rather than name or
  path.
- `open <index>` to open the file with the given index in your default
  application.
- `quit` or `exit` will exit out of the interactive mode.

### `.fstore` files
