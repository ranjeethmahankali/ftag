use crate::query::safe_get_flag;

#[derive(Debug)]
pub(crate) enum FilterParseError {
    EmptyQuery,
    MalformedParens,
    ExpectedBinaryOperator,
    UnexpectedBinaryOperator,
    EndOfTokens,
}

pub(crate) trait TagData: std::fmt::Display + Clone + Default {}

impl TagData for String {}

impl TagData for usize {}

pub(crate) trait TagMaker<T: TagData> {
    fn create_tag(&self, input: &str) -> Filter<T>;
}

struct StringMaker;

impl TagMaker<String> for StringMaker {
    fn create_tag(&self, input: &str) -> Filter<String> {
        return Tag(input.to_string());
    }
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
    pub fn eval_vec(&self, flags: &Vec<bool>) -> bool {
        match self {
            Tag(ti) => safe_get_flag(flags, *ti),
            And(lhs, rhs) => lhs.eval_vec(flags) && rhs.eval_vec(flags),
            Or(lhs, rhs) => lhs.eval_vec(flags) || rhs.eval_vec(flags),
            Not(input) => !input.eval_vec(flags),
            FalseTag => false,
            TrueTag => true,
        }
    }

    pub fn eval_slice(&self, flags: &[bool]) -> bool {
        match self {
            Tag(ti) => flags[*ti],
            And(lhs, rhs) => lhs.eval_slice(flags) && rhs.eval_slice(flags),
            Or(lhs, rhs) => lhs.eval_slice(flags) || rhs.eval_slice(flags),
            Not(input) => !input.eval_slice(flags),
            FalseTag => false,
            TrueTag => true,
        }
    }

    pub fn to_string(&self, tagnames: &[String]) -> String {
        match self {
            Tag(i) => tagnames[*i].clone(),
            And(lhs, rhs) => format!(
                "{} & {}",
                Self::maybe_parens(self, lhs, lhs.to_string(tagnames)),
                Self::maybe_parens(self, rhs, rhs.to_string(tagnames))
            ),
            Or(lhs, rhs) => format!(
                "{} | {}",
                Self::maybe_parens(self, lhs, lhs.to_string(tagnames)),
                Self::maybe_parens(self, rhs, rhs.to_string(tagnames))
            ),
            Not(filter) => format!(
                "!{}",
                Self::maybe_parens(self, filter, filter.to_string(tagnames))
            ),
            FalseTag => String::from("FALSE_TAG"),
            TrueTag => String::from("TRUE_TAG"),
        }
    }
}

impl Filter<String> {
    #[cfg(test)]
    pub fn to_string(&self) -> String {
        match self {
            Tag(tag) => tag.clone(),
            And(lhs, rhs) => format!(
                "{} & {}",
                Self::maybe_parens(self, lhs, lhs.to_string()),
                Self::maybe_parens(self, rhs, rhs.to_string())
            ),
            Or(lhs, rhs) => format!(
                "{} | {}",
                Self::maybe_parens(self, lhs, lhs.to_string()),
                Self::maybe_parens(self, rhs, rhs.to_string()),
            ),
            Not(filter) => format!("!{}", Self::maybe_parens(self, filter, filter.to_string())),
            FalseTag => String::from("FALSE_TAG"),
            TrueTag => String::from("TRUE_TAG"),
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
        FalseTag => TrueTag,
        TrueTag => FalseTag,
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
        tokens.push(Token::Parsed(tagmaker.create_tag(&input[from..to])));
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
