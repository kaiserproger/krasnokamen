//! The JSON reader behind the payload encoder. It keeps each number's text and
//! each object's key order: vanilla's codecs round a JSON number from its
//! decimal text (`Float.parseFloat`, `Double.parseDouble`), and `serde_json`'s
//! default float parsing is not correctly rounded for 17-digit doubles.

use thiserror::Error;

/// Nesting deeper than any data-pack entry; the NBT limit is 512.
const MAX_DEPTH: usize = 256;

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Json<'a> {
    Null,
    Bool(bool),
    /// The number's text as written, valid JSON number grammar.
    Number(&'a str),
    String(String),
    Array(Vec<Json<'a>>),
    /// Members in document order; keys are unique.
    Object(Vec<(String, Json<'a>)>),
}

#[derive(Debug, Error, PartialEq, Eq)]
#[error("byte {offset}: {reason}")]
pub struct JsonError {
    offset: usize,
    reason: &'static str,
}

pub(crate) fn parse(text: &str) -> Result<Json<'_>, JsonError> {
    let mut reader = Reader { text, at: 0 };
    let value = reader.value(0)?;
    reader.skip_whitespace();
    if reader.at != text.len() {
        return Err(reader.error("trailing characters after the value"));
    }
    Ok(value)
}

/// Whether a JSON number's text is an integer (no fraction or exponent).
pub(crate) fn is_integer(number: &str) -> bool {
    !number.contains(['.', 'e', 'E'])
}

struct Reader<'a> {
    text: &'a str,
    at: usize,
}

impl<'a> Reader<'a> {
    fn error(&self, reason: &'static str) -> JsonError {
        JsonError {
            offset: self.at,
            reason,
        }
    }

    fn peek(&self) -> Option<u8> {
        self.text.as_bytes().get(self.at).copied()
    }

    fn skip_whitespace(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.at += 1;
        }
    }

    fn expect(&mut self, literal: &'static str) -> Result<(), JsonError> {
        if self.text[self.at..].starts_with(literal) {
            self.at += literal.len();
            Ok(())
        } else {
            Err(self.error("unexpected character"))
        }
    }

    fn value(&mut self, depth: usize) -> Result<Json<'a>, JsonError> {
        if depth > MAX_DEPTH {
            return Err(self.error("nesting too deep"));
        }
        self.skip_whitespace();
        match self.peek() {
            Some(b'{') => self.object(depth),
            Some(b'[') => self.array(depth),
            Some(b'"') => self.string().map(Json::String),
            Some(b't') => self.expect("true").map(|()| Json::Bool(true)),
            Some(b'f') => self.expect("false").map(|()| Json::Bool(false)),
            Some(b'n') => self.expect("null").map(|()| Json::Null),
            Some(b'-' | b'0'..=b'9') => self.number().map(Json::Number),
            Some(_) => Err(self.error("unexpected character")),
            None => Err(self.error("unexpected end of input")),
        }
    }

    fn object(&mut self, depth: usize) -> Result<Json<'a>, JsonError> {
        self.at += 1;
        let mut members: Vec<(String, Json<'a>)> = Vec::new();
        self.skip_whitespace();
        if self.peek() == Some(b'}') {
            self.at += 1;
            return Ok(Json::Object(members));
        }
        loop {
            self.skip_whitespace();
            if self.peek() != Some(b'"') {
                return Err(self.error("expected a member name"));
            }
            let key = self.string()?;
            if members.iter().any(|(existing, _)| *existing == key) {
                return Err(self.error("duplicate member name"));
            }
            self.skip_whitespace();
            self.expect(":")?;
            let value = self.value(depth + 1)?;
            members.push((key, value));
            self.skip_whitespace();
            match self.peek() {
                Some(b',') => self.at += 1,
                Some(b'}') => {
                    self.at += 1;
                    return Ok(Json::Object(members));
                }
                _ => return Err(self.error("expected ',' or '}'")),
            }
        }
    }

    fn array(&mut self, depth: usize) -> Result<Json<'a>, JsonError> {
        self.at += 1;
        let mut items = Vec::new();
        self.skip_whitespace();
        if self.peek() == Some(b']') {
            self.at += 1;
            return Ok(Json::Array(items));
        }
        loop {
            items.push(self.value(depth + 1)?);
            self.skip_whitespace();
            match self.peek() {
                Some(b',') => self.at += 1,
                Some(b']') => {
                    self.at += 1;
                    return Ok(Json::Array(items));
                }
                _ => return Err(self.error("expected ',' or ']'")),
            }
        }
    }

    /// `-? (0 | [1-9][0-9]*) (. [0-9]+)? ([eE] [+-]? [0-9]+)?`
    fn number(&mut self) -> Result<&'a str, JsonError> {
        let start = self.at;
        if self.peek() == Some(b'-') {
            self.at += 1;
        }
        match self.peek() {
            Some(b'0') => self.at += 1,
            Some(b'1'..=b'9') => self.digits(),
            _ => return Err(self.error("invalid number")),
        }
        if self.peek() == Some(b'.') {
            self.at += 1;
            self.required_digits()?;
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.at += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.at += 1;
            }
            self.required_digits()?;
        }
        Ok(&self.text[start..self.at])
    }

    fn digits(&mut self) {
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.at += 1;
        }
    }

    fn required_digits(&mut self) -> Result<(), JsonError> {
        let start = self.at;
        self.digits();
        if self.at == start {
            return Err(self.error("invalid number"));
        }
        Ok(())
    }

    fn string(&mut self) -> Result<String, JsonError> {
        self.at += 1;
        let mut out = String::new();
        loop {
            let run_start = self.at;
            while let Some(byte) = self.peek() {
                if byte == b'"' || byte == b'\\' || byte < 0x20 {
                    break;
                }
                self.at += 1;
            }
            out.push_str(&self.text[run_start..self.at]);
            match self.peek() {
                Some(b'"') => {
                    self.at += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.at += 1;
                    out.push(self.escape()?);
                }
                Some(_) => return Err(self.error("unescaped control character in a string")),
                None => return Err(self.error("unterminated string")),
            }
        }
    }

    fn escape(&mut self) -> Result<char, JsonError> {
        let Some(byte) = self.peek() else {
            return Err(self.error("unterminated escape"));
        };
        self.at += 1;
        Ok(match byte {
            b'"' => '"',
            b'\\' => '\\',
            b'/' => '/',
            b'b' => '\u{8}',
            b'f' => '\u{c}',
            b'n' => '\n',
            b'r' => '\r',
            b't' => '\t',
            b'u' => {
                let unit = self.hex_unit()?;
                if (0xD800..0xDC00).contains(&unit) {
                    self.expect("\\u")?;
                    let low = self.hex_unit()?;
                    if !(0xDC00..0xE000).contains(&low) {
                        return Err(self.error("unpaired surrogate"));
                    }
                    let scalar = 0x10000 + ((unit - 0xD800) << 10) + (low - 0xDC00);
                    char::from_u32(scalar).ok_or_else(|| self.error("invalid code point"))?
                } else {
                    char::from_u32(unit).ok_or_else(|| self.error("unpaired surrogate"))?
                }
            }
            _ => return Err(self.error("invalid escape")),
        })
    }

    fn hex_unit(&mut self) -> Result<u32, JsonError> {
        let digits = self
            .text
            .get(self.at..self.at + 4)
            .filter(|digits| digits.bytes().all(|byte| byte.is_ascii_hexdigit()))
            .ok_or_else(|| self.error("invalid \\u escape"))?;
        self.at += 4;
        u32::from_str_radix(digits, 16).map_err(|_| self.error("invalid \\u escape"))
    }
}
