//! Hand-written EDN and JSON emission and a minimal EDN reader.
//!
//! Done by hand (no serde) so `propsim-history` has zero required dependencies
//! and `cargo tree` for a default-feature build pulls nothing (architecture.md
//! §17). EDN is the default wire for the Elle path; JSON is for generic interop.
//!
//! The reader is intentionally minimal: it parses the subset of EDN that this
//! crate emits and that Jepsen `store/` histories use — maps, vectors, keywords,
//! strings, integers, floats, booleans, and `nil`. It is not a general EDN parser
//! (no tagged literals, sets, chars, ratios, or symbols beyond keywords).

use crate::op::Value;
use crate::ParseError;

// ---------------------------------------------------------------------------
// Writing
// ---------------------------------------------------------------------------

/// Append the EDN rendering of `v` to `out`.
pub(crate) fn write_edn(out: &mut String, v: &Value) {
    match v {
        Value::Nil => out.push_str("nil"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Int(i) => out.push_str(&i.to_string()),
        Value::Float(f) => out.push_str(&fmt_float(*f)),
        Value::Str(s) => write_quoted(out, s),
        Value::Keyword(k) => {
            out.push(':');
            out.push_str(k);
        }
        Value::List(items) => {
            out.push('[');
            for (i, it) in items.iter().enumerate() {
                if i > 0 {
                    out.push(' ');
                }
                write_edn(out, it);
            }
            out.push(']');
        }
        Value::Map(pairs) => {
            out.push('{');
            for (i, (k, val)) in pairs.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                write_edn(out, k);
                out.push(' ');
                write_edn(out, val);
            }
            out.push('}');
        }
    }
}

/// Append the JSON rendering of `v` to `out`.
///
/// Keywords become strings prefixed with `:` (round-trippable back to keywords),
/// matching how Jepsen/`elle-cli` JSON represents them. Map keys are coerced to
/// strings (JSON requires string keys).
pub(crate) fn write_json(out: &mut String, v: &Value) {
    match v {
        Value::Nil => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Int(i) => out.push_str(&i.to_string()),
        Value::Float(f) => out.push_str(&fmt_float(*f)),
        Value::Str(s) => write_quoted(out, s),
        Value::Keyword(k) => {
            // ":kw" as a JSON string so it can be recovered on parse.
            out.push('"');
            out.push(':');
            push_escaped(out, k);
            out.push('"');
        }
        Value::List(items) => {
            out.push('[');
            for (i, it) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_json(out, it);
            }
            out.push(']');
        }
        Value::Map(pairs) => {
            out.push('{');
            for (i, (k, val)) in pairs.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_json_key(out, k);
                out.push(':');
                write_json(out, val);
            }
            out.push('}');
        }
    }
}

fn write_json_key(out: &mut String, k: &Value) {
    match k {
        Value::Str(s) => write_quoted(out, s),
        Value::Keyword(kw) => {
            out.push('"');
            out.push(':');
            push_escaped(out, kw);
            out.push('"');
        }
        Value::Int(i) => {
            out.push('"');
            out.push_str(&i.to_string());
            out.push('"');
        }
        other => {
            // Fall back to the value's string form, quoted.
            let mut tmp = String::new();
            write_edn(&mut tmp, other);
            write_quoted(out, &tmp);
        }
    }
}

fn fmt_float(f: f64) -> String {
    if f.is_finite() {
        // Ensure a decimal point so it reads back as a float, not an int.
        let s = format!("{f}");
        if s.contains('.') || s.contains('e') || s.contains('E') {
            s
        } else {
            format!("{s}.0")
        }
    } else {
        // EDN/JSON have no NaN/Inf literal; emit nil rather than invalid output.
        "nil".to_string()
    }
}

fn write_quoted(out: &mut String, s: &str) {
    out.push('"');
    push_escaped(out, s);
    out.push('"');
}

fn push_escaped(out: &mut String, s: &str) {
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(c),
        }
    }
}

// ---------------------------------------------------------------------------
// Reading (minimal EDN)
// ---------------------------------------------------------------------------

/// Parse a single EDN value from `s`, requiring the whole string to be consumed
/// (apart from trailing whitespace).
pub(crate) fn parse_edn(s: &str) -> Result<Value, ParseError> {
    let mut p = Parser::new(s);
    p.skip_ws();
    let v = p.parse_value()?;
    p.skip_ws();
    if p.pos != p.bytes.len() {
        return Err(ParseError::new(format!(
            "trailing input at byte {} ",
            p.pos
        )));
    }
    Ok(v)
}

struct Parser<'a> {
    bytes: &'a [u8],
    src: &'a str,
    pos: usize,
}

impl<'a> Parser<'a> {
    fn new(s: &'a str) -> Self {
        Parser {
            bytes: s.as_bytes(),
            src: s,
            pos: 0,
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn skip_ws(&mut self) {
        // EDN treats commas as whitespace; also skip `;` line comments.
        loop {
            match self.peek() {
                Some(b) if b.is_ascii_whitespace() || b == b',' => self.pos += 1,
                Some(b';') => {
                    while let Some(c) = self.peek() {
                        self.pos += 1;
                        if c == b'\n' {
                            break;
                        }
                    }
                }
                _ => break,
            }
        }
    }

    fn parse_value(&mut self) -> Result<Value, ParseError> {
        self.skip_ws();
        match self.peek() {
            None => Err(ParseError::new("unexpected end of input")),
            Some(b'{') => self.parse_map(),
            Some(b'[') | Some(b'(') => self.parse_seq(),
            Some(b'"') => self.parse_string().map(Value::Str),
            Some(b':') => self.parse_keyword(),
            Some(_) => self.parse_atom(),
        }
    }

    fn parse_map(&mut self) -> Result<Value, ParseError> {
        self.pos += 1; // consume '{'
        let mut pairs = Vec::new();
        loop {
            self.skip_ws();
            match self.peek() {
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(Value::Map(pairs));
                }
                None => return Err(ParseError::new("unterminated map")),
                _ => {
                    let k = self.parse_value()?;
                    let v = self.parse_value()?;
                    pairs.push((k, v));
                }
            }
        }
    }

    fn parse_seq(&mut self) -> Result<Value, ParseError> {
        let close = if self.peek() == Some(b'(') {
            b')'
        } else {
            b']'
        };
        self.pos += 1; // consume opener
        let mut items = Vec::new();
        loop {
            self.skip_ws();
            match self.peek() {
                Some(c) if c == close => {
                    self.pos += 1;
                    return Ok(Value::List(items));
                }
                None => return Err(ParseError::new("unterminated list/vector")),
                _ => items.push(self.parse_value()?),
            }
        }
    }

    fn parse_string(&mut self) -> Result<String, ParseError> {
        self.pos += 1; // consume opening quote
        let mut out = String::new();
        loop {
            match self.peek() {
                None => return Err(ParseError::new("unterminated string")),
                Some(b'"') => {
                    self.pos += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.pos += 1;
                    match self.peek() {
                        Some(b'"') => out.push('"'),
                        Some(b'\\') => out.push('\\'),
                        Some(b'n') => out.push('\n'),
                        Some(b'r') => out.push('\r'),
                        Some(b't') => out.push('\t'),
                        Some(other) => out.push(other as char),
                        None => return Err(ParseError::new("trailing escape")),
                    }
                    self.pos += 1;
                }
                Some(_) => {
                    // Copy one UTF-8 char.
                    let ch = self.next_char();
                    out.push(ch);
                }
            }
        }
    }

    fn parse_keyword(&mut self) -> Result<Value, ParseError> {
        self.pos += 1; // consume ':'
        let start = self.pos;
        while let Some(c) = self.peek() {
            if is_delim(c) {
                break;
            }
            self.pos += 1;
        }
        if self.pos == start {
            return Err(ParseError::new("empty keyword"));
        }
        Ok(Value::Keyword(self.src[start..self.pos].to_string()))
    }

    fn parse_atom(&mut self) -> Result<Value, ParseError> {
        let start = self.pos;
        while let Some(c) = self.peek() {
            if is_delim(c) {
                break;
            }
            self.pos += 1;
        }
        let tok = &self.src[start..self.pos];
        match tok {
            "nil" => Ok(Value::Nil),
            "true" => Ok(Value::Bool(true)),
            "false" => Ok(Value::Bool(false)),
            _ => {
                if let Ok(i) = tok.parse::<i64>() {
                    return Ok(Value::Int(i));
                }
                // Strip a trailing EDN float suffix like `1.0M`/`1.0N` before
                // trying f64.
                let f_tok = tok.trim_end_matches(['M', 'N']);
                if let Ok(f) = f_tok.parse::<f64>() {
                    return Ok(Value::Float(f));
                }
                Err(ParseError::new(format!("unrecognized token `{tok}`")))
            }
        }
    }

    fn next_char(&mut self) -> char {
        let ch = self.src[self.pos..].chars().next().unwrap();
        self.pos += ch.len_utf8();
        ch
    }
}

fn is_delim(c: u8) -> bool {
    c.is_ascii_whitespace()
        || matches!(
            c,
            b',' | b'{' | b'}' | b'[' | b']' | b'(' | b')' | b'"' | b';'
        )
}

// ---------------------------------------------------------------------------
// Reading (minimal JSON)
// ---------------------------------------------------------------------------

/// Parse a single JSON value from `s`, requiring the whole string be consumed.
///
/// Keywords are recovered from strings of the form `":name"` (how
/// [`write_json`] emits them), so a JSON history round-trips back to the same
/// keyword-bearing [`Value`]s.
pub(crate) fn parse_json(s: &str) -> Result<Value, ParseError> {
    let mut p = JsonParser::new(s);
    p.skip_ws();
    let v = p.parse_value()?;
    p.skip_ws();
    if p.pos != p.bytes.len() {
        return Err(ParseError::new(format!(
            "trailing JSON input at byte {}",
            p.pos
        )));
    }
    Ok(v)
}

struct JsonParser<'a> {
    bytes: &'a [u8],
    src: &'a str,
    pos: usize,
}

impl<'a> JsonParser<'a> {
    fn new(s: &'a str) -> Self {
        JsonParser {
            bytes: s.as_bytes(),
            src: s,
            pos: 0,
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn skip_ws(&mut self) {
        while let Some(b) = self.peek() {
            if b.is_ascii_whitespace() {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn parse_value(&mut self) -> Result<Value, ParseError> {
        self.skip_ws();
        match self.peek() {
            None => Err(ParseError::new("unexpected end of JSON")),
            Some(b'{') => self.parse_object(),
            Some(b'[') => self.parse_array(),
            Some(b'"') => Ok(str_to_value(self.parse_string()?)),
            Some(b't') | Some(b'f') | Some(b'n') => self.parse_keyword_literal(),
            Some(_) => self.parse_number(),
        }
    }

    fn parse_object(&mut self) -> Result<Value, ParseError> {
        self.pos += 1; // '{'
        let mut pairs = Vec::new();
        loop {
            self.skip_ws();
            match self.peek() {
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(Value::Map(pairs));
                }
                Some(b',') => {
                    self.pos += 1;
                }
                Some(b'"') => {
                    let key = str_to_value(self.parse_string()?);
                    self.skip_ws();
                    if self.peek() != Some(b':') {
                        return Err(ParseError::new("expected `:` after JSON key"));
                    }
                    self.pos += 1;
                    let val = self.parse_value()?;
                    pairs.push((key, val));
                }
                _ => return Err(ParseError::new("malformed JSON object")),
            }
        }
    }

    fn parse_array(&mut self) -> Result<Value, ParseError> {
        self.pos += 1; // '['
        let mut items = Vec::new();
        loop {
            self.skip_ws();
            match self.peek() {
                Some(b']') => {
                    self.pos += 1;
                    return Ok(Value::List(items));
                }
                Some(b',') => self.pos += 1,
                None => return Err(ParseError::new("unterminated JSON array")),
                _ => items.push(self.parse_value()?),
            }
        }
    }

    fn parse_string(&mut self) -> Result<String, ParseError> {
        self.pos += 1; // opening quote
        let mut out = String::new();
        loop {
            match self.peek() {
                None => return Err(ParseError::new("unterminated JSON string")),
                Some(b'"') => {
                    self.pos += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.pos += 1;
                    match self.peek() {
                        Some(b'"') => out.push('"'),
                        Some(b'\\') => out.push('\\'),
                        Some(b'/') => out.push('/'),
                        Some(b'n') => out.push('\n'),
                        Some(b'r') => out.push('\r'),
                        Some(b't') => out.push('\t'),
                        Some(b'u') => {
                            let cp = self.parse_unicode_escape()?;
                            out.push(cp);
                            continue;
                        }
                        Some(other) => out.push(other as char),
                        None => return Err(ParseError::new("trailing JSON escape")),
                    }
                    self.pos += 1;
                }
                Some(_) => {
                    let ch = self.src[self.pos..].chars().next().unwrap();
                    self.pos += ch.len_utf8();
                    out.push(ch);
                }
            }
        }
    }

    fn parse_unicode_escape(&mut self) -> Result<char, ParseError> {
        self.pos += 1; // 'u'
        if self.pos + 4 > self.bytes.len() {
            return Err(ParseError::new("truncated \\u escape"));
        }
        let hex = &self.src[self.pos..self.pos + 4];
        let code =
            u32::from_str_radix(hex, 16).map_err(|_| ParseError::new("invalid \\u escape"))?;
        self.pos += 4;
        char::from_u32(code).ok_or_else(|| ParseError::new("invalid code point"))
    }

    fn parse_keyword_literal(&mut self) -> Result<Value, ParseError> {
        for (lit, val) in [
            ("true", Value::Bool(true)),
            ("false", Value::Bool(false)),
            ("null", Value::Nil),
        ] {
            if self.src[self.pos..].starts_with(lit) {
                self.pos += lit.len();
                return Ok(val);
            }
        }
        Err(ParseError::new("invalid JSON literal"))
    }

    fn parse_number(&mut self) -> Result<Value, ParseError> {
        let start = self.pos;
        while let Some(c) = self.peek() {
            if matches!(c, b'0'..=b'9' | b'-' | b'+' | b'.' | b'e' | b'E') {
                self.pos += 1;
            } else {
                break;
            }
        }
        let tok = &self.src[start..self.pos];
        if let Ok(i) = tok.parse::<i64>() {
            return Ok(Value::Int(i));
        }
        tok.parse::<f64>()
            .map(Value::Float)
            .map_err(|_| ParseError::new(format!("invalid JSON number `{tok}`")))
    }
}

/// Recover a keyword from a `":name"` string; otherwise keep it a string.
fn str_to_value(s: String) -> Value {
    if let Some(rest) = s.strip_prefix(':') {
        Value::Keyword(rest.to_string())
    } else {
        Value::Str(s)
    }
}
