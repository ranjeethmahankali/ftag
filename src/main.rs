mod core;
mod filter;
mod interactive;
mod query;

use crate::{
    core::{get_all_tags, get_store_path, untracked_files, FstoreError, Info},
    query::run_query,
};
use clap::{command, value_parser, Arg};
use std::path::PathBuf;

fn main() -> Result<(), FstoreError> {
    let current_dir = std::env::current_dir().map_err(|_| FstoreError::InvalidWorkingDirectory)?;
    let matches = parse_args();
    if let Some(matches) = matches.subcommand_matches(cmd::QUERY) {
        return run_query(
            current_dir,
            matches
                .get_one::<String>(arg::FILTER)
                .ok_or(FstoreError::InvalidArgs)?,
        );
    } else if let Some(_matches) = matches.subcommand_matches(cmd::INTERACTIVE) {
        return interactive::start();
    } else if let Some(_matches) = matches.subcommand_matches(cmd::CHECK) {
        return core::check(current_dir);
    } else if let Some(matches) = matches.subcommand_matches(cmd::WHATIS) {
        match matches.get_one::<PathBuf>(arg::PATH) {
            Some(path) => {
                let Info { tags, desc } = core::what_is(path)?;
                let tagstr = {
                    let mut tags = tags.into_iter();
                    let first = tags.next().unwrap_or(String::new());
                    tags.fold(first, |acc, t| format!("{}, {}", acc, t))
                };
                println!(
                    "tags: [{}]{}",
                    tagstr,
                    if desc.is_empty() {
                        desc
                    } else {
                        format!("\n\n{}", desc)
                    }
                );
                return Ok(());
            }
            None => return Err(FstoreError::InvalidArgs),
        }
    } else if let Some(matches) = matches.subcommand_matches(cmd::EDIT) {
        let path = matches
            .get_one::<PathBuf>(arg::PATH)
            .unwrap_or(&current_dir);
        std::process::Command::new(
            std::env::var("EDITOR").unwrap_or(default_editor()?.to_string()),
        )
        .args([get_store_path::<false>(&path).ok_or(FstoreError::InvalidPath(path.clone()))?])
        .status()
        .map_err(|_| FstoreError::InternalError)?;
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
        return Err(FstoreError::InvalidArgs);
    }
}

fn default_editor() -> Result<&'static str, FstoreError> {
    match std::env::consts::OS {
        "linux" => Ok("nano"),
        "windows" => Ok("notepad"),
        _ => return Err(FstoreError::NoDefaultEditor),
    }
}

fn parse_args() -> clap::ArgMatches {
    command!()
        .subcommand(
            clap::Command::new(cmd::QUERY)
                .alias("-q")
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
        .get_matches()
}

mod cmd {
    pub const QUERY: &str = "query";
    pub const INTERACTIVE: &str = "interactive";
    pub const CHECK: &str = "check";
    pub const WHATIS: &str = "whatis";
    pub const EDIT: &str = "edit";
    pub const UNTRACKED: &str = "untracked";
    pub const TAGS: &str = "tags";
}

mod arg {
    pub const FILTER: &str = "filter";
    pub const PATH: &str = "path";
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
    pub const CHECK: &str = "Recursively traverse directories starting from the working directory and check to see if all the files listed in every .fstore file is exists.";
    pub const CHECK_PATH:&str = "The directory path where to start checking recursively. If ommitted, the workind directory is assumed.";
    pub const WHATIS: &str = "Get the tags and description (if found) of the given file.";
    pub const WHATIS_PATH: &str = "Path of the file to describe.";
    pub const EDIT: &str = "Edit the .fstore file of the given (optional) directory.
If the environment variable EDITOR is set, it will be used to open the file. If it is not set, fstore can try to guess your default editor, but this is not guaranteed to work. Setting the EDITOR environment variable is recommended.";
    pub const EDIT_PATH: &str = "Path to the directory whose .fstore file you wish to edit. If no path is specified, the current working
directory is used as default.";
    pub const UNTRACKED: &str =
        "List all files that are not tracked by fstore, recursively from the current directory.";
    pub const TAGS: &str = "List all tags found by traversing the directories recursively from the current directory. The output list of tags will not contain duplicates.";
}
