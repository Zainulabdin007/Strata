//! Strata Query Language parser (not SQL).

use crate::error::{Error, Result};
use crate::plan::{Literal, Predicate, Query};

/// Parses a Strata query string into a typed [`Query`].
///
/// See `QUERY.md` for grammar. Keywords are case-insensitive; field names are
/// case-sensitive.
pub fn parse_query(input: &str) -> Result<Query> {
    let tokens = tokenize(input)?;
    let mut p = Parser {
        tokens: &tokens,
        pos: 0,
    };
    p.parse_query()
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Ident(String),
    String(String),
    Number(String),
    Comma,
    LParen,
    RParen,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Eof,
}

struct Parser<'a> {
    tokens: &'a [Tok],
    pos: usize,
}

impl Parser<'_> {
    fn peek(&self) -> &Tok {
        self.tokens.get(self.pos).unwrap_or(&Tok::Eof)
    }

    fn bump(&mut self) -> Tok {
        let t = self.tokens.get(self.pos).cloned().unwrap_or(Tok::Eof);
        if self.pos < self.tokens.len() {
            self.pos += 1;
        }
        t
    }

    fn expect_kw(&mut self, kw: &str) -> Result<()> {
        match self.bump() {
            Tok::Ident(s) if s.eq_ignore_ascii_case(kw) => Ok(()),
            other => Err(Error::parse(format!("expected `{kw}`, got {other:?}"))),
        }
    }

    fn parse_query(&mut self) -> Result<Query> {
        self.expect_kw("select")?;
        let select = self.parse_select_list()?;

        let mut predicates = Vec::new();
        if matches!(self.peek(), Tok::Ident(s) if s.eq_ignore_ascii_case("where")) {
            self.bump();
            predicates = self.parse_predicates()?;
        }

        let mut limit = None;
        if matches!(self.peek(), Tok::Ident(s) if s.eq_ignore_ascii_case("limit")) {
            self.bump();
            limit = Some(self.parse_u64_literal()?);
        }

        if !matches!(self.peek(), Tok::Eof) {
            return Err(Error::parse(format!(
                "unexpected trailing token {:?}",
                self.peek()
            )));
        }

        Ok(Query {
            select,
            predicates,
            limit,
        })
    }

    fn parse_select_list(&mut self) -> Result<Vec<String>> {
        if matches!(self.peek(), Tok::Ident(s) if s == "*") {
            self.bump();
            return Ok(Vec::new());
        }
        let mut cols = Vec::new();
        loop {
            cols.push(self.parse_ident()?);
            if matches!(self.peek(), Tok::Comma) {
                self.bump();
                continue;
            }
            break;
        }
        Ok(cols)
    }

    fn parse_predicates(&mut self) -> Result<Vec<Predicate>> {
        let mut preds = vec![self.parse_predicate()?];
        while matches!(self.peek(), Tok::Ident(s) if s.eq_ignore_ascii_case("and")) {
            self.bump();
            preds.push(self.parse_predicate()?);
        }
        Ok(preds)
    }

    fn parse_predicate(&mut self) -> Result<Predicate> {
        let field = self.parse_ident()?;
        match self.peek() {
            Tok::Ident(s) if s.eq_ignore_ascii_case("between") => {
                self.bump();
                let low = self.parse_literal()?;
                if matches!(self.peek(), Tok::Ident(s) if s.eq_ignore_ascii_case("and")) {
                    self.bump();
                } else {
                    return Err(Error::parse("expected `and` in BETWEEN"));
                }
                let high = self.parse_literal()?;
                Ok(Predicate::Between { field, low, high })
            }
            Tok::Ident(s) if s.eq_ignore_ascii_case("in") => {
                self.bump();
                self.expect_tok(&Tok::LParen)?;
                let mut values = Vec::new();
                loop {
                    values.push(self.parse_literal()?);
                    if matches!(self.peek(), Tok::Comma) {
                        self.bump();
                        continue;
                    }
                    break;
                }
                self.expect_tok(&Tok::RParen)?;
                Ok(Predicate::In { field, values })
            }
            Tok::Eq => {
                self.bump();
                Ok(Predicate::Eq {
                    field,
                    value: self.parse_literal()?,
                })
            }
            Tok::Ne => {
                self.bump();
                Ok(Predicate::Ne {
                    field,
                    value: self.parse_literal()?,
                })
            }
            Tok::Lt => {
                self.bump();
                Ok(Predicate::Lt {
                    field,
                    value: self.parse_literal()?,
                })
            }
            Tok::Le => {
                self.bump();
                Ok(Predicate::Le {
                    field,
                    value: self.parse_literal()?,
                })
            }
            Tok::Gt => {
                self.bump();
                Ok(Predicate::Gt {
                    field,
                    value: self.parse_literal()?,
                })
            }
            Tok::Ge => {
                self.bump();
                Ok(Predicate::Ge {
                    field,
                    value: self.parse_literal()?,
                })
            }
            other => Err(Error::parse(format!(
                "expected comparison operator after `{field}`, got {other:?}"
            ))),
        }
    }

    fn expect_tok(&mut self, want: &Tok) -> Result<()> {
        let got = self.bump();
        if &got == want {
            Ok(())
        } else {
            Err(Error::parse(format!("expected {want:?}, got {got:?}")))
        }
    }

    fn parse_ident(&mut self) -> Result<String> {
        match self.bump() {
            Tok::Ident(s) => Ok(s),
            other => Err(Error::parse(format!("expected identifier, got {other:?}"))),
        }
    }

    fn parse_u64_literal(&mut self) -> Result<u64> {
        match self.parse_literal()? {
            Literal::U64(n) => Ok(n),
            Literal::I64(n) if n >= 0 => Ok(n as u64),
            other => Err(Error::parse(format!(
                "expected unsigned integer, got {}",
                other.display()
            ))),
        }
    }

    fn parse_literal(&mut self) -> Result<Literal> {
        match self.bump() {
            Tok::Ident(s) => {
                if s.eq_ignore_ascii_case("null") {
                    Ok(Literal::Null)
                } else if s.eq_ignore_ascii_case("true") {
                    Ok(Literal::Bool(true))
                } else if s.eq_ignore_ascii_case("false") {
                    Ok(Literal::Bool(false))
                } else {
                    // Bare word → string (AAPL, buy, …)
                    Ok(Literal::String(s))
                }
            }
            Tok::String(s) => Ok(Literal::String(s)),
            Tok::Number(s) => parse_number(&s),
            other => Err(Error::parse(format!("expected literal, got {other:?}"))),
        }
    }
}

fn parse_number(s: &str) -> Result<Literal> {
    if let Ok(u) = s.parse::<u64>() {
        // Prefer U64 for non-negative; also fits timestamps that fit u64.
        // Signed-looking negatives handled below.
        if !s.starts_with('-') {
            return Ok(Literal::U64(u));
        }
    }
    if let Ok(i) = s.parse::<i64>() {
        return Ok(Literal::I64(i));
    }
    Err(Error::parse(format!("invalid number `{s}`")))
}

fn tokenize(input: &str) -> Result<Vec<Tok>> {
    let mut tokens = Vec::new();
    let bytes = input.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        let c = bytes[i] as char;
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if c == '#' {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        match c {
            ',' => {
                tokens.push(Tok::Comma);
                i += 1;
            }
            '(' => {
                tokens.push(Tok::LParen);
                i += 1;
            }
            ')' => {
                tokens.push(Tok::RParen);
                i += 1;
            }
            '=' if bytes.get(i + 1) == Some(&b'=') => {
                tokens.push(Tok::Eq);
                i += 2;
            }
            '=' => {
                tokens.push(Tok::Eq);
                i += 1;
            }
            '!' if bytes.get(i + 1) == Some(&b'=') => {
                tokens.push(Tok::Ne);
                i += 2;
            }
            '<' if bytes.get(i + 1) == Some(&b'=') => {
                tokens.push(Tok::Le);
                i += 2;
            }
            '<' => {
                tokens.push(Tok::Lt);
                i += 1;
            }
            '>' if bytes.get(i + 1) == Some(&b'=') => {
                tokens.push(Tok::Ge);
                i += 2;
            }
            '>' => {
                tokens.push(Tok::Gt);
                i += 1;
            }
            '"' | '\'' => {
                let quote = bytes[i];
                i += 1;
                let start = i;
                while i < bytes.len() && bytes[i] != quote {
                    i += 1;
                }
                if i >= bytes.len() {
                    return Err(Error::parse("unterminated string literal"));
                }
                let s = std::str::from_utf8(&bytes[start..i])
                    .map_err(|_| Error::parse("string literal is not UTF-8"))?
                    .to_string();
                tokens.push(Tok::String(s));
                i += 1;
            }
            _ if c == '-' || c.is_ascii_digit() => {
                let start = i;
                i += 1;
                while i < bytes.len() && (bytes[i] as char).is_ascii_digit() {
                    i += 1;
                }
                let s = std::str::from_utf8(&bytes[start..i])
                    .map_err(|_| Error::parse("number is not UTF-8"))?
                    .to_string();
                if s == "-" {
                    return Err(Error::parse("lonely minus sign"));
                }
                tokens.push(Tok::Number(s));
            }
            _ if c == '*' || c.is_ascii_alphabetic() || c == '_' => {
                let start = i;
                i += 1;
                while i < bytes.len() {
                    let ch = bytes[i] as char;
                    if ch.is_ascii_alphanumeric() || ch == '_' {
                        i += 1;
                    } else {
                        break;
                    }
                }
                let s = std::str::from_utf8(&bytes[start..i])
                    .map_err(|_| Error::parse("identifier is not UTF-8"))?
                    .to_string();
                tokens.push(Tok::Ident(s));
            }
            _ => {
                return Err(Error::parse(format!("unexpected character `{c}`")));
            }
        }
    }
    tokens.push(Tok::Eof);
    Ok(tokens)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_example_query() {
        let q = parse_query(
            r#"
            select ts, price, quantity
            where symbol == AAPL
              and ts between 100 and 200
            limit 50
            "#,
        )
        .unwrap();
        assert_eq!(q.select, vec!["ts", "price", "quantity"]);
        assert_eq!(q.predicates.len(), 2);
        assert_eq!(q.limit, Some(50));
    }

    #[test]
    fn parses_in_list() {
        let q = parse_query("select * where symbol in (AAPL, MSFT, \"GOOG\")").unwrap();
        match &q.predicates[0] {
            Predicate::In { values, .. } => assert_eq!(values.len(), 3),
            _ => panic!("expected IN"),
        }
    }
}
