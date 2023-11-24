#[derive(Debug)]
pub(crate) enum FilterParseError {
    EmptyQuery,
    MalformedParens,
    ExpectedBinaryOperator,
    UnexpectedBinaryOperator,
    EndOfTokens,
}

pub(crate) trait TagData: std::fmt::Display + Clone {}

impl TagData for String {}

#[derive(Clone)]
pub(crate) struct TagIndex {
    pub value: Option<usize>,
}

impl std::fmt::Display for TagIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.value {
            Some(i) => write!(f, "{}", i),
            None => write!(f, "None"),
        }
    }
}

impl TagData for TagIndex {}

pub(crate) trait TagMaker<T: TagData> {
    fn create_tag(&self, input: &str) -> T;
}

struct StringMaker;

impl TagMaker<String> for StringMaker {
    fn create_tag(&self, input: &str) -> String {
        return input.to_string();
    }
}

#[derive(Debug)]
pub(crate) enum Filter<T: TagData> {
    Tag(T),
    And(Box<Filter<T>>, Box<Filter<T>>),
    Or(Box<Filter<T>>, Box<Filter<T>>),
    Not(Box<Filter<T>>),
}

use Filter::*;

use crate::query::safe_get_flag;

impl<T: TagData> Filter<T> {
    pub fn parse(input: &str, tagmaker: &impl TagMaker<T>) -> Result<Self, FilterParseError> {
        parse_filter(input, tagmaker)
    }
}

impl Filter<TagIndex> {
    pub fn evaluate(&self, flags: &Vec<bool>) -> bool {
        match self {
            Tag(ti) => match ti.value {
                Some(i) => safe_get_flag(flags, i),
                None => false,
            },
            And(lhs, rhs) => lhs.evaluate(flags) && rhs.evaluate(flags),
            Or(lhs, rhs) => lhs.evaluate(flags) || rhs.evaluate(flags),
            Not(input) => !input.evaluate(flags),
        }
    }
}

impl Filter<String> {
    #[cfg(test)]
    pub fn to_string(&self) -> String {
        match self {
            Tag(tag) => tag.clone(),
            And(lhs, rhs) => format!("{} & {}", self.maybe_parens(lhs), self.maybe_parens(rhs)),
            Or(lhs, rhs) => format!("{} | {}", self.maybe_parens(lhs), self.maybe_parens(rhs)),
            Not(filter) => format!("!{}", self.maybe_parens(filter)),
        }
    }

    #[cfg(test)]
    fn maybe_parens(&self, filter: &Filter<String>) -> String {
        match (filter, self) {
            (Tag(_), _) | (Not(_), _) | (And(_, _), And(_, _)) | (Or(_, _), Or(_, _)) => {
                filter.to_string()
            }
            _ => format!("({})", filter.to_string()),
        }
    }
}

#[derive(Debug)]
enum Token<T: TagData> {
    And,
    Or,
    Not,
    Parsed(Filter<T>),
}

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
    let mut iter = input.char_indices();
    while let Some((i, c)) = iter.next() {
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
    push_tag(input, begin, end + 1, &mut stack, tagmaker);
    return parse_tokens(stack.drain(..));
}

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
    return Ok(filter);
}

fn next_filter<T: TagData, I: Iterator<Item = Token<T>>>(
    iter: &mut I,
) -> Result<Filter<T>, FilterParseError> {
    match iter.next() {
        Some(t) => match t {
            Token::And | Token::Or => Err(FilterParseError::UnexpectedBinaryOperator),
            Token::Not => Ok(not_filter(next_filter(iter)?)),
            Token::Parsed(filter) => Ok(filter),
        },
        None => Err(FilterParseError::EndOfTokens),
    }
}

fn not_filter<T: TagData>(filter: Filter<T>) -> Filter<T> {
    match filter {
        Tag(_) | And(_, _) | Or(_, _) => Filter::Not(Box::new(filter)),
        Not(inner) => *inner,
    }
}

fn push_tag<T: TagData>(
    input: &str,
    from: usize,
    to: usize,
    tokens: &mut Vec<Token<T>>,
    tagmaker: &impl TagMaker<T>,
) {
    if to > from {
        tokens.push(Token::Parsed(Filter::Tag(
            tagmaker.create_tag(&input[from..to]),
        )));
    }
}

#[cfg(test)]
mod test {
    use super::*;

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
