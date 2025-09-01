use ftag::{
    core::{self, Error, get_all_tags, search, untracked_files},
    load::get_ftag_path,
    query::{TagTable, count_files_tags, run_query},
};
use std::{path::PathBuf, str::FromStr};

fn main() -> Result<(), Error> {
    let Arguments {
        path: current_dir,
        command,
    } = parse_args();
    match command {
        Command::BashComplete(items) => handle_bash_completions(current_dir, items),
        Command::Count => {
            let (nfiles, ntags) = count_files_tags(current_dir)?;
            println!("{nfiles} files; {ntags} tags")
        }
        Command::Query(filter) => run_query(current_dir, &filter)?,
        Command::Search(needle) => search(current_dir, &needle)?,
        Command::Interactive => ftag::tui::start(TagTable::from_dir(current_dir)?)
            .map_err(|err| Error::TUIFailure(format!("{err:?}")))?,
        Command::Check => core::check(current_dir)?,
        Command::WhatIs(path) => println!("{}", core::what_is(&path)?),
        Command::Edit(path) => {
            let path = path.as_ref().unwrap_or(&current_dir);
            ftag::open::edit_file(match get_ftag_path::<false>(path) {
                Some(fpath) => Ok(fpath),
                None => Err(Error::InvalidPath(path.clone())),
            }?)
            .map_err(|e| Error::EditCommandFailed(format!("{e:?}")))?
        }
        Command::Clean => core::clean(current_dir)?,
        Command::Untracked => {
            for path in untracked_files(current_dir)? {
                println!("{}", path.display());
            }
        }
        Command::Tags => {
            let mut tags: Box<[String]> = get_all_tags(current_dir)?.collect();
            tags.sort_unstable();
            for tag in tags {
                println!("{tag}");
            }
        }
        Command::Help => println!("{}", HELP_TEXT),
        Command::Version => println!("ftag CLI: {}", env!("CARGO_PKG_VERSION")),
    };
    Ok(())
}

fn handle_bash_completions(current_dir: PathBuf, mut words: Vec<String>) {
    /*
    Bash completion always passes in 3 words. The first word will be the main
    binary: ftag. The second word will be an empty string, and the third word
    will be the most recent word that the user typed, that provides the context
    for completions.
     */
    if words.len() != 3 || words[0] != "ftag" {
        return;
    }
    const PREV_WORDS: [&str; 11] = [
        "query",
        "-q",
        "interactive",
        "check",
        "whatis",
        "edit",
        "untracked",
        "tags",
        "clean",
        "--path",
        "-p",
    ];
    let last = words.pop();
    match last.as_deref() {
        Some("ftag") => {
            if let Some(cmd) = words.pop() {
                for suggestion in PREV_WORDS.iter().filter(|c| c.starts_with(&cmd)) {
                    println!("{suggestion}");
                }
            }
        }
        Some("query") | Some("-q") => {
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
                for tag in tags.filter(|t| t.starts_with(right)) {
                    println!("{left}{tag}");
                }
            }
        }
        _ => {} // Defer to default bash completion for files and directories.
    }
}

#[derive(Debug)]
enum Command {
    BashComplete(Vec<String>),
    Count,
    Query(String),
    Search(String),
    Interactive,
    Check,
    WhatIs(PathBuf),
    Edit(Option<PathBuf>),
    Clean,
    Untracked,
    Tags,
    Help,
    Version,
}

#[derive(Debug)]
struct Arguments {
    path: PathBuf,
    command: Command,
}

fn parse_args() -> Arguments {
    let mut path: Option<PathBuf> = None; // Default choice of path.
    let mut cmdopt: Option<Command> = None;
    let mut args = std::env::args().skip(1); // First argument is the executable.
    while let Some(word) = args.next() {
        match (word.as_str(), &cmdopt, &path) {
            ("--bash-complete", None, _) => {
                cmdopt = Some(Command::BashComplete(args.by_ref().collect()))
            }
            ("count", None, _) => cmdopt = Some(Command::Count),
            ("query" | "-q", None, _) => {
                cmdopt = Some(Command::Query(
                    args.next()
                        .expect("ERROR: Cannot find the query filter argument."),
                ))
            }
            ("search" | "-s", None, _) => {
                cmdopt = Some(Command::Search(
                    args.next()
                        .expect("ERROR: Cannot find argument for the 'search' command"),
                ));
            }
            ("interactive" | "-i", None, _) => cmdopt = Some(Command::Interactive),
            ("check", None, _) => cmdopt = Some(Command::Check),
            ("whatis", None, _) => {
                cmdopt = Some(Command::WhatIs(
                    PathBuf::from_str(
                        &args
                            .next()
                            .expect("ERROR: Cannot read argument of 'whatis' command."),
                    )
                    .inspect_err(|e| eprintln!("ERROR: {}", e))
                    .expect("ERROR: Cannot parse argument of 'whatis' command.")
                    .canonicalize()
                    .inspect_err(|e| eprintln!("ERROR: {}", e))
                    .expect("ERROR: Unable to parse the argument of 'whatis' command."),
                ))
            }
            ("edit", None, _) => {
                cmdopt = Some(Command::Edit(args.next().map(|p| {
                    PathBuf::from_str(&p)
                        .inspect_err(|e| eprintln!("ERROR: {}", e))
                        .expect("ERROR: Unable to parse the argument of 'edit' command.")
                        .canonicalize()
                        .inspect_err(|e| eprintln!("ERROR: {}", e))
                        .expect("ERROR: Cannot parse the argument of 'edit' command.")
                })))
            }
            ("clean", None, _) => cmdopt = Some(Command::Clean),
            ("untracked", None, _) => cmdopt = Some(Command::Untracked),
            ("tags", None, _) => cmdopt = Some(Command::Tags),
            ("help" | "--help" | "-h" | "?", None, _) => cmdopt = Some(Command::Help),
            ("version" | "--version", None, _) => cmdopt = Some(Command::Version),
            ("--path" | "-p", _, None) => {
                path = Some(
                    PathBuf::from_str(
                        &args
                            .next()
                            .expect("ERROR: No value found for the 'path' argument"),
                    )
                    .inspect_err(|e| eprintln!("ERROR: {}", e))
                    .expect("ERROR: Cannot parse the value of the 'path' argument")
                    .canonicalize()
                    .inspect_err(|e| eprintln!("ERROR: {}", e))
                    .expect("ERROR: Cannot canonicalize path"),
                );
            }
            _ => {
                eprintln!("ERROR: Unexpected argument: {}", &word);
                panic!("ABORTED.")
            }
        }
    }
    Arguments {
        path: match path {
            Some(path) => path,
            None => std::env::current_dir()
                .inspect_err(|e| eprintln!("ERROR: {}", e))
                .expect("ERROR: Cannot get working directory"),
        },
        command: match cmdopt {
            Some(command) => command,
            None => {
                eprintln!("ERROR: Command not found among arguments");
                panic!("ABORTED.")
            }
        },
    }
}

const HELP_TEXT: &str = r#"ftag - CLI tool for tagging and searching files

  USAGE:
      ftag [OPTIONS] <COMMAND> [ARGS...]

  OPTIONS:
      -p, --path <PATH>    Run command in specified directory instead of current directory
      -h, --help          Show this help message
      --version           Show version information

  COMMANDS:
      count               Output the number of tracked files

      query, -q <FILTER>  List all files that match the given query string
                          The query string must be composed of tags and supported boolean operations:
                          & (for and), | (for or) and ! (for not). An example query string is
                          'foo & bar'. Using this will list all files that have both tags 'foo'
                          and 'bar'. More complex queries can be delimited using parentheses.
                          For example: '(foo & bar) | !baz' will list all files that either have
                          both 'foo' and 'bar' tags, or don't have the 'baz' tag.

      search, -s <KEYWORDS>
                          Search all tags and descriptions for the given keywords. Any file that
                          contains any of the keywords in this string in either its tags or
                          description will be included in the output.

      interactive, -i     Launch interactive mode in the working directory. Interactive mode loads
                          all the files and tags, and lets you incrementally refine your search
                          criteria inside a TUI.

      check               Recursively traverse directories starting from the working directory and
                          check to see if all the files listed in every .ftag file exist.

      whatis <PATH>       Get the tags and description (if found) of the given file.

      edit [PATH]         Edit the .ftag file of the given (optional) directory. If the environment
                          variable EDITOR is set, it will be used to open the file. If it is not set,
                          ftag can try to guess your default editor, but this is not guaranteed to
                          work. Setting the EDITOR environment variable is recommended. If no path is
                          specified, the current working directory is used as default.

      clean               Clean all the tag data. This includes deleting globs that don't match to
                          any files on the disk, and merging globs that share the same tags and
                          description into the same entry.

      untracked           List all files that are not tracked by ftag, recursively from the current
                          directory.

      tags                List all tags found by traversing the directories recursively from the
                          current directory. The output list of tags will not contain duplicates.

  EXAMPLES:
      ftag count                          # Count files in current directory
      ftag -p /path/to/dir tags          # List all tags in specified directory
      ftag query "rust & !test"          # Find rust files that aren't tests
      ftag search "documentation"        # Search for files containing "documentation"
      ftag interactive                   # Launch TUI mode
      ftag whatis src/main.rs           # Show tags for specific file
      ftag edit                         # Edit .ftag file for current directory
  "#;
