mod core;
mod filter;
mod interactive;
mod load;
mod query;
mod walk;

use crate::{
    core::{get_all_tags, untracked_files, Error},
    query::run_query,
};
use clap::{command, value_parser, Arg};
use load::get_store_path;
use query::DenseTagTable;
use std::path::PathBuf;

fn main() -> Result<(), Error> {
    let matches = parse_args();
    let current_dir = if let Some(rootdir) = matches.get_one::<PathBuf>("path") {
        rootdir
            .canonicalize()
            .map_err(|_| Error::InvalidPath(rootdir.clone()))?
    } else {
        std::env::current_dir().map_err(|_| Error::InvalidWorkingDirectory)?
    };
    // Handle tab completions first.
    if let Some(complete) = matches.subcommand_matches(cmd::BASH_COMPLETE) {
        // Bash completions can be registered with:
        // complete -o default -C 'ftag --bash-complete --' ftag
        if let Some(words) = complete.get_many::<String>(arg::BASH_COMPLETE_WORDS) {
            handle_bash_completions(current_dir, words.map(|s| s.as_str()).collect());
        }
        return Ok(());
    }
    if let Some(matches) = matches.subcommand_matches(cmd::QUERY) {
        return run_query(
            current_dir,
            matches
                .get_one::<String>(arg::FILTER)
                .ok_or(Error::InvalidArgs)?,
        );
    } else if let Some(_matches) = matches.subcommand_matches(cmd::INTERACTIVE) {
        return interactive::start(DenseTagTable::from_dir(current_dir)?)
            .map_err(|err| Error::TUIError(format!("{:?}", err)));
    } else if let Some(_matches) = matches.subcommand_matches(cmd::CHECK) {
        return core::check(current_dir);
    } else if let Some(matches) = matches.subcommand_matches(cmd::WHATIS) {
        match matches.get_one::<PathBuf>(arg::PATH) {
            Some(path) => {
                let path = path
                    .canonicalize()
                    .map_err(|_| Error::InvalidPath(path.clone()))?;
                println!("{}", core::what_is(&path)?);
                return Ok(());
            }
            None => return Err(Error::InvalidArgs),
        }
    } else if let Some(matches) = matches.subcommand_matches(cmd::EDIT) {
        let path = matches
            .get_one::<PathBuf>(arg::PATH)
            .unwrap_or(&current_dir);
        edit::edit_file(get_store_path::<false>(path).ok_or(Error::InvalidPath(path.clone()))?)
            .map_err(|e| Error::EditCommandFailed(format!("{:?}", e)))?;
        return Ok(());
    } else if let Some(_matches) = matches.subcommand_matches(cmd::UNTRACKED) {
        for path in untracked_files(current_dir)? {
            println!("{}", path.display());
        }
        return Ok(());
    } else if let Some(_matches) = matches.subcommand_matches(cmd::TAGS) {
        println!("{}", get_all_tags(current_dir)?.join("\n"));
        return Ok(());
    } else {
        return Err(Error::InvalidArgs);
    }
}

fn handle_bash_completions(current_dir: PathBuf, mut words: Vec<&str>) {
    if words.len() != 3 {
        return;
    }
    if words[0] != "ftag" {
        return;
    }
    const PREV_WORDS: [&str; 10] = [
        "query",
        "-q",
        "interactive",
        "check",
        "whatis",
        "edit",
        "untracked",
        "tags",
        "--path",
        "-p",
    ];
    match words.pop() {
        Some("ftag") => {
            if let Some(cmd) = words.pop() {
                for suggestion in PREV_WORDS.iter().filter(|c| c.starts_with(cmd)) {
                    println!("{}", suggestion);
                }
            }
        }
        Some(cmd::QUERY) | Some(cmd::QUERY_SHORT) => {
            if let (Some(word), Ok(tags)) = (words.pop(), get_all_tags(current_dir)) {
                let (left, right) = {
                    let mut last = 0usize;
                    for (i, c) in word.char_indices() {
                        match c {
                            '|' | '(' | ')' | '&' | '!' => last = i,
                            _ if c.is_whitespace() => last = i,
                            _ => {} // Do nothing.
                        }
                    }
                    let last = if last == 0 { last } else { last + 1 };
                    (&word[..last], &word[last..])
                };
                for tag in tags.iter().filter(|t| t.starts_with(&right)) {
                    println!("{left}{}", tag);
                }
            }
        }
        _ => {} // Defer to default bash completion for files and directories..
    }
}

fn parse_args() -> clap::ArgMatches {
    command!()
        .arg(
            Arg::new(arg::PATH)
                .long("path")
                .short('p')
                .required(false)
                .value_parser(value_parser!(PathBuf)),
        )
        .subcommand(
            clap::Command::new(cmd::QUERY)
                .alias(cmd::QUERY_SHORT)
                .about(about::QUERY)
                .arg(
                    Arg::new(arg::FILTER)
                        .required(true)
                        .help(about::QUERY_FILTER)
                        .long_help(about::QUERY_FILTER_LONG),
                ),
        )
        .subcommand(
            clap::Command::new(cmd::INTERACTIVE)
                .alias("-i")
                .about(about::INTERACTIVE),
        )
        .subcommand(
            clap::Command::new(cmd::CHECK).about(about::CHECK).arg(
                Arg::new(arg::PATH)
                    .help(about::CHECK_PATH)
                    .required(false)
                    .value_parser(value_parser!(PathBuf)),
            ),
        )
        .subcommand(
            clap::Command::new(cmd::WHATIS).about(about::WHATIS).arg(
                Arg::new(arg::PATH)
                    .required(true)
                    .value_parser(value_parser!(PathBuf))
                    .help(about::WHATIS_PATH),
            ),
        )
        .subcommand(
            clap::Command::new(cmd::EDIT).about(about::EDIT).arg(
                Arg::new(arg::PATH)
                    .help(about::EDIT_PATH)
                    .required(false)
                    .value_parser(value_parser!(PathBuf))
                    .default_value("."),
            ),
        )
        .subcommand(clap::Command::new(cmd::UNTRACKED).about(about::UNTRACKED))
        .subcommand(clap::Command::new(cmd::TAGS).about(about::TAGS))
        .subcommand(
            clap::Command::new(cmd::BASH_COMPLETE)
                .arg(Arg::new(arg::BASH_COMPLETE_WORDS).num_args(3)),
        )
        .get_matches()
}

mod cmd {
    pub const QUERY: &str = "query";
    pub const QUERY_SHORT: &str = "-q";
    pub const INTERACTIVE: &str = "interactive";
    pub const CHECK: &str = "check";
    pub const WHATIS: &str = "whatis";
    pub const EDIT: &str = "edit";
    pub const UNTRACKED: &str = "untracked";
    pub const TAGS: &str = "tags";
    pub const BASH_COMPLETE: &str = "--bash-complete";
}

mod arg {
    pub const FILTER: &str = "filter";
    pub const PATH: &str = "path";
    pub const BASH_COMPLETE_WORDS: &str = "bash-complete-words";
}

mod about {
    pub const QUERY: &str = "List all files that match the given query string.";
    pub const QUERY_FILTER: &str = "The query string to compare the files against.";
    pub const QUERY_FILTER_LONG: &str =
        "The query string must be composed of tags and supported boolean operations:
& (for and), | (for or) and ! (for not).  An example query
string is 'foo & bar'. Using this will list all files that have both
tags 'foo' and 'bar'.  More complex queries can be delimited using
parentheses. For example: '(foo & bar) | !baz' will list all files
that either have both 'foo' and 'bar' tags, or don't have the 'baz'
tag.";
    pub const INTERACTIVE: &str = "\
Launch interactive mode in the working directory.
// TODO Add more docs later after the interactive mode is implemented.";
    pub const CHECK: &str = "Recursively traverse directories starting from the working directory and check to see if all the files listed in every .ftag file is exists.";
    pub const CHECK_PATH:&str = "The directory path where to start checking recursively. If ommitted, the workind directory is assumed.";
    pub const WHATIS: &str = "Get the tags and description (if found) of the given file.";
    pub const WHATIS_PATH: &str = "Path of the file to describe.";
    pub const EDIT: &str = "Edit the .ftag file of the given (optional) directory.
If the environment variable EDITOR is set, it will be used to open the file. If it is not set, ftag can try to guess your default editor, but this is not guaranteed to work. Setting the EDITOR environment variable is recommended.";
    pub const EDIT_PATH: &str = "Path to the directory whose .ftag file you wish to edit. If no path is specified, the current working
directory is used as default.";
    pub const UNTRACKED: &str =
        "List all files that are not tracked by ftag, recursively from the current directory.";
    pub const TAGS: &str = "List all tags found by traversing the directories recursively from the current directory. The output list of tags will not contain duplicates.";
}
