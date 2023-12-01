# fstore

fstore is a tool that lets you attach tags and descriptions to files,
then later query and retrieve files based on their tags.

## Installation

Fstore is written in Rust. It is not available as a crate yet, but you
can clone this repo install it using cargo by running:

```bash
cargo install --path /path/to/fstore/repo/
```

## Usage

## Bash autocompletion

Add this to your bash profile:
```bash
complete -o default -C 'fstore --bash-complete' fstore
```
