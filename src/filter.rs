use crate::query::safe_get_flag;
use std::fmt::{Debug, Display};

pub(crate) enum FilterParseError {
    EmptyQuery,
    MalformedParens,
    ExpectedBinaryOperator,
    UnexpectedBinaryOperator(String),
    EndOfTokens,
}

impl Debug for FilterParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FilterParseError::EmptyQuery => write!(f, "The filter string is empty."),
            FilterParseError::MalformedParens => write!(f, "Parentheses are unbalanced."),
            FilterParseError::ExpectedBinaryOperator => write!(f, "A binary operator is missing."),
            FilterParseError::UnexpectedBinaryOperator(t) => write!(f, "'{}' was not expected.", t),
            FilterParseError::EndOfTokens => write!(f, "Unexpected end of tokens."),
        }
    }
}

/// Data representing a tag. Tags are usually strings, so this can be
/// a string. But sometimes it can be more efficient to represent tags
/// as indices into a list / table of strings.
pub(crate) trait TagData: std::fmt::Display + Clone + Default {}

impl TagData for usize {}

/// The user always supplies tags as strings. But `TagData` can be
/// other things such as indices. A class that implements this trait
/// should know how to convert a user supplied tag string into a
/// filter that wraps `TagData`.
pub(crate) trait TagMaker<T: TagData> {
    fn create_tag(&self, input: &str) -> Filter<T>;
}

#[derive(Debug)]
pub(crate) enum Filter<T: TagData> {
    Tag(T),
    And(Box<Filter<T>>, Box<Filter<T>>),
    Or(Box<Filter<T>>, Box<Filter<T>>),
    Not(Box<Filter<T>>),
    FalseTag, // always false.
    TrueTag,  // Always true.
}
use Filter::*;

impl<T: TagData> Default for Filter<T> {
    fn default() -> Self {
        Tag(T::default())
    }
}

impl<T: TagData> Filter<T> {
    pub fn parse(input: &str, tagmaker: &impl TagMaker<T>) -> Result<Self, FilterParseError> {
        parse_filter(input, tagmaker)
    }

    fn maybe_parens(parent: &Filter<T>, child: &Filter<T>, childstr: String) -> String {
        match (child, parent) {
            (Tag(_), _) | (Not(_), _) | (And(_, _), And(_, _)) | (Or(_, _), Or(_, _)) => childstr,
            _ => format!("({})", childstr),
        }
    }
}

impl Filter<usize> {
    pub fn eval(&self, flags: &[bool]) -> bool {
        match self {
            Tag(ti) => safe_get_flag(flags, *ti),
            And(lhs, rhs) => lhs.eval(flags) && rhs.eval(flags),
            Or(lhs, rhs) => lhs.eval(flags) || rhs.eval(flags),
            Not(input) => !input.eval(flags),
            FalseTag => false,
            TrueTag => true,
        }
    }

    pub fn text(&self, tagnames: &[String]) -> String {
        match self {
            Tag(i) => tagnames[*i].clone(),
            And(lhs, rhs) => format!(
                "{} & {}",
                Self::maybe_parens(self, lhs, lhs.text(tagnames)),
                Self::maybe_parens(self, rhs, rhs.text(tagnames))
            ),
            Or(lhs, rhs) => format!(
                "{} | {}",
                Self::maybe_parens(self, lhs, lhs.text(tagnames)),
                Self::maybe_parens(self, rhs, rhs.text(tagnames))
            ),
            Not(filter) => format!(
                "!{}",
                Self::maybe_parens(self, filter, filter.text(tagnames))
            ),
            FalseTag => String::from("FALSE_TAG"),
            TrueTag => String::from("TRUE_TAG"),
        }
    }
}

impl<T: TagData> Display for Filter<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Tag(tag) => write!(f, "{}", tag),
            And(lhs, rhs) => write!(
                f,
                "{} & {}",
                Self::maybe_parens(self, lhs, lhs.to_string()),
                Self::maybe_parens(self, rhs, rhs.to_string())
            ),
            Or(lhs, rhs) => write!(
                f,
                "{} | {}",
                Self::maybe_parens(self, lhs, lhs.to_string()),
                Self::maybe_parens(self, rhs, rhs.to_string()),
            ),
            Not(filter) => write!(
                f,
                "!{}",
                Self::maybe_parens(self, filter, filter.to_string())
            ),
            FalseTag => write!(f, "FALSE_TAG"),
            TrueTag => write!(f, "TRUE_TAG"),
        }
    }
}

enum Token<T: TagData> {
    And,
    Or,
    Not,
    Parsed(Filter<T>),
}

impl<T: TagData> Display for Token<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Token::And => write!(f, "&"),
            Token::Or => write!(f, "|"),
            Token::Not => write!(f, "!"),
            Token::Parsed(p) => write!(f, "{}", p),
        }
    }
}

/// Parse filter from a string. The tagmaker is used to create tag-data from
/// strings corresponding to the tags.
fn parse_filter<T: TagData>(
    input: &str,
    tagmaker: &impl TagMaker<T>,
) -> Result<Filter<T>, FilterParseError> {
    if input.is_empty() {
        return Err(FilterParseError::EmptyQuery);
    }
    let mut stack: Vec<Token<T>> = Vec::new();
    let mut parens: Vec<usize> = Vec::new();
    let mut begin: usize = 0;
    let mut end = 0;
    for (i, c) in input.char_indices() {
        end = i;
        match c {
            '(' => {
                parens.push(stack.len());
                begin = i + 1;
                continue;
            }
            ')' => {
                push_tag(input, begin, i, &mut stack, tagmaker);
                begin = i + 1;
                let last = parens.pop().ok_or(FilterParseError::MalformedParens)?;
                if last >= stack.len() - 1 {
                    continue;
                }
                let filter = parse_tokens(stack.drain(last..))?;
                stack.truncate(last);
                stack.push(Token::Parsed(filter));
            }
            '!' => {
                push_tag(input, begin, i, &mut stack, tagmaker);
                begin = i + 1;
                stack.push(Token::Not);
            }
            '&' => {
                push_tag(input, begin, i, &mut stack, tagmaker);
                begin = i + 1;
                stack.push(Token::And);
            }
            '|' => {
                push_tag(input, begin, i, &mut stack, tagmaker);
                begin = i + 1;
                stack.push(Token::Or);
            }
            _ if c.is_whitespace() => {
                push_tag(input, begin, i, &mut stack, tagmaker);
                begin = i + 1;
            }
            _ => {}
        };
    }
    if !parens.is_empty() {
        return Err(FilterParseError::MalformedParens);
    }
    push_tag(input, begin, end + 1, &mut stack, tagmaker);
    return parse_tokens(stack.drain(..));
}

/// Reduce the iterator of tokens into a filter.
fn parse_tokens<T: TagData, I: Iterator<Item = Token<T>>>(
    mut iter: I,
) -> Result<Filter<T>, FilterParseError> {
    let mut filter = next_filter(&mut iter)?;
    while let Some(t) = iter.next() {
        filter = match t {
            Token::And => Filter::And(Box::new(filter), Box::new(next_filter(&mut iter)?)),
            Token::Or => Filter::Or(Box::new(filter), Box::new(next_filter(&mut iter)?)),
            Token::Not | Token::Parsed(_) => return Err(FilterParseError::ExpectedBinaryOperator),
        };
    }
    Ok(filter)
}

/// Get the next filter from a list of tokens.
fn next_filter<T: TagData, I: Iterator<Item = Token<T>>>(
    iter: &mut I,
) -> Result<Filter<T>, FilterParseError> {
    match iter.next() {
        Some(t) => match t {
            Token::And | Token::Or => {
                Err(FilterParseError::UnexpectedBinaryOperator(t.to_string()))
            }
            Token::Not => Ok(not_filter(next_filter(iter)?)),
            Token::Parsed(filter) => Ok(filter),
        },
        None => Err(FilterParseError::EndOfTokens),
    }
}

/// Instead of simply wrapping a filter in a `not` filter, this will
/// check if the given filter is already a not filter and fold
/// `!!something` into `something`.
fn not_filter<T: TagData>(filter: Filter<T>) -> Filter<T> {
    match filter {
        Tag(_) | And(_, _) | Or(_, _) => Filter::Not(Box::new(filter)),
        Not(inner) => *inner,
        FalseTag => TrueTag,
        TrueTag => FalseTag,
    }
}

/// Push the tag into the vector of tokens. The tag-data is created using the
/// tag maker.
fn push_tag<T: TagData>(
    input: &str,
    from: usize,
    to: usize,
    tokens: &mut Vec<Token<T>>,
    tagmaker: &impl TagMaker<T>,
) {
    if to > from {
        tokens.push(Token::Parsed(tagmaker.create_tag(&input[from..to])));
    }
}

#[cfg(test)]
mod test {
    use super::*;

    struct StringMaker;

    impl TagData for String {}

    impl TagMaker<String> for StringMaker {
        fn create_tag(&self, input: &str) -> Filter<String> {
            Tag(input.to_string())
        }
    }

    #[test]
    fn t_filter_parse_round_trip() {
        let maker = StringMaker;
        for fstr in [
            "apple & banana",
            "(apple & mango) | banana",
            "(apple & mango) | !banana",
            "(apple & pear) | !(banana & !pear) | (fig & grape)",
        ] {
            assert_eq!(Filter::parse(fstr, &maker).unwrap().to_string(), fstr);
        }
    }

    #[test]
    fn t_not_not_filter() {
        let maker = StringMaker;
        for (before, after) in [
            ("!apple", "!apple"),
            ("!!apple", "apple"),
            ("!!!apple", "!apple"),
            ("!!!!apple", "apple"),
            ("!!(!apple)", "!apple"),
            ("!!(!!(!apple))", "!apple"),
            ("!!(!!(!(!apple)))", "apple"),
        ] {
            assert_eq!(Filter::parse(before, &maker).unwrap().to_string(), after);
        }
    }
}
