use std::fmt::{Debug, Display};

pub enum FilterParseError {
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

#[derive(Debug, PartialEq, Eq)]
pub enum Filter {
    Tag(usize),
    And(Box<Filter>, Box<Filter>),
    Or(Box<Filter>, Box<Filter>),
    Not(Box<Filter>),
    FalseTag, // always false.
    TrueTag,  // Always true.
}
use Filter::*;

impl Filter {
    pub fn parse<F>(input: &str, mut tagmaker: F) -> Result<Self, FilterParseError>
    where
        F: FnMut(&str) -> Filter,
    {
        parse_filter(input, &mut tagmaker)
    }

    fn maybe_parens(parent: &Filter, child: &Filter, childstr: String) -> String {
        match (child, parent) {
            (Tag(_), _) | (Not(_), _) | (And(_, _), And(_, _)) | (Or(_, _), Or(_, _)) => childstr,
            _ => format!("({})", childstr),
        }
    }

    pub fn eval<F>(&self, checker: F) -> bool
    where
        F: Fn(usize) -> bool,
    {
        self.eval_impl(&checker)
    }

    fn eval_impl<F>(&self, checker: &F) -> bool
    where
        F: Fn(usize) -> bool,
    {
        match self {
            Tag(ti) => checker(*ti),
            And(lhs, rhs) => lhs.eval_impl(checker) && rhs.eval_impl(checker),
            Or(lhs, rhs) => lhs.eval_impl(checker) || rhs.eval_impl(checker),
            Not(input) => !input.eval_impl(checker),
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
            FalseTag => String::from("NOT_A_TAG"),
            TrueTag => String::from("ALL_TAGS"),
        }
    }
}

impl Display for Filter {
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

enum Token {
    And,
    Or,
    Not,
    Parsed(Filter),
}

impl Display for Token {
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
fn parse_filter<F>(input: &str, tagmaker: &mut F) -> Result<Filter, FilterParseError>
where
    F: FnMut(&str) -> Filter,
{
    if input.is_empty() {
        return Err(FilterParseError::EmptyQuery);
    }
    let mut stack: Vec<Token> = Vec::new();
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
    parse_tokens(stack.into_iter())
}

/// Reduce the iterator of tokens into a filter.
fn parse_tokens<I: Iterator<Item = Token>>(mut iter: I) -> Result<Filter, FilterParseError> {
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
fn next_filter<I: Iterator<Item = Token>>(iter: &mut I) -> Result<Filter, FilterParseError> {
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
fn not_filter(filter: Filter) -> Filter {
    match filter {
        Tag(_) | And(_, _) | Or(_, _) => Filter::Not(Box::new(filter)),
        Not(inner) => *inner,
        FalseTag => TrueTag,
        TrueTag => FalseTag,
    }
}

/// Push the tag into the vector of tokens. The tag-data is created using the
/// tag maker.
fn push_tag<F>(input: &str, from: usize, to: usize, tokens: &mut Vec<Token>, tagmaker: &mut F)
where
    F: FnMut(&str) -> Filter,
{
    if to > from {
        tokens.push(Token::Parsed(tagmaker(&input[from..to])));
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn t_filter_parse_round_trip() {
        for fstr in [
            "apple & banana",
            "(apple & mango) | banana",
            "(apple & mango) | !banana",
            "(apple & pear) | !(banana & !pear) | (fig & grape)",
        ] {
            let mut map = BTreeMap::<String, usize>::new();
            let filter = Filter::parse(fstr, |tag| {
                let size = map.len();
                let idx = *map.entry(tag.to_string()).or_insert(size);
                Filter::Tag(idx)
            })
            .unwrap();
            let tagnames: Box<[_]> = {
                let mut pairs: Vec<_> = map.into_iter().collect();
                pairs.sort_by(|(_ta, ia), (_tb, ib)| ia.cmp(ib));
                pairs.into_iter().map(|(t, _i)| t).collect()
            };
            assert_eq!(filter.text(&tagnames), fstr);
        }
    }

    #[test]
    fn t_not_not_filter() {
        for (before, after) in [
            ("!apple", "!apple"),
            ("!!apple", "apple"),
            ("!!!apple", "!apple"),
            ("!!!!apple", "apple"),
            ("!!(!apple)", "!apple"),
            ("!!(!!(!apple))", "!apple"),
            ("!!(!!(!(!apple)))", "apple"),
        ] {
            let mut map = BTreeMap::<String, usize>::new();
            let filter = Filter::parse(before, |tag| {
                let size = map.len();
                Filter::Tag(*map.entry(tag.to_string()).or_insert(size))
            })
            .unwrap();
            let tagnames: Box<[_]> = {
                let mut pairs: Vec<_> = map.into_iter().collect();
                pairs.sort_by(|(_ta, ia), (_tb, ib)| ia.cmp(ib));
                pairs.into_iter().map(|(t, _i)| t).collect()
            };
            assert_eq!(filter.text(&tagnames), after);
        }
    }
}
