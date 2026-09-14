//! Just enough JSON to read `compile_commands.json` and write the report,
//! without pulling in a runtime dependency.

use std::fmt::Write as _;

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<Value>),
    Object(Vec<(String, Value)>),
}

impl Value {
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Object(fields) => fields.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Value::Array(items) => Some(items),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Number(n) => Some(*n),
            _ => None,
        }
    }
}

pub fn parse(text: &str) -> Result<Value, String> {
    let mut p = Parser {
        s: text.as_bytes(),
        pos: 0,
    };
    p.ws();
    let v = p.value(0)?;
    p.ws();
    if p.pos != p.s.len() {
        return Err(p.err("trailing characters after JSON value"));
    }
    Ok(v)
}

struct Parser<'a> {
    s: &'a [u8],
    pos: usize,
}

const MAX_DEPTH: usize = 256;

impl Parser<'_> {
    fn err(&self, msg: &str) -> String {
        let line = 1 + self.s[..self.pos.min(self.s.len())]
            .iter()
            .filter(|&&c| c == b'\n')
            .count();
        format!("invalid JSON at line {line}: {msg}")
    }

    fn ws(&mut self) {
        while self.pos < self.s.len() && matches!(self.s[self.pos], b' ' | b'\t' | b'\n' | b'\r') {
            self.pos += 1;
        }
    }

    fn eat(&mut self, lit: &[u8]) -> bool {
        if self.s[self.pos..].starts_with(lit) {
            self.pos += lit.len();
            true
        } else {
            false
        }
    }

    fn value(&mut self, depth: usize) -> Result<Value, String> {
        if depth > MAX_DEPTH {
            return Err(self.err("nesting too deep"));
        }
        match self.s.get(self.pos) {
            None => Err(self.err("unexpected end of input")),
            Some(b'{') => {
                self.pos += 1;
                let mut fields = Vec::new();
                self.ws();
                if self.eat(b"}") {
                    return Ok(Value::Object(fields));
                }
                loop {
                    self.ws();
                    if self.s.get(self.pos) != Some(&b'"') {
                        return Err(self.err("expected string key"));
                    }
                    let key = self.string()?;
                    self.ws();
                    if !self.eat(b":") {
                        return Err(self.err("expected ':'"));
                    }
                    self.ws();
                    let v = self.value(depth + 1)?;
                    fields.push((key, v));
                    self.ws();
                    if self.eat(b",") {
                        continue;
                    }
                    if self.eat(b"}") {
                        return Ok(Value::Object(fields));
                    }
                    return Err(self.err("expected ',' or '}'"));
                }
            }
            Some(b'[') => {
                self.pos += 1;
                let mut items = Vec::new();
                self.ws();
                if self.eat(b"]") {
                    return Ok(Value::Array(items));
                }
                loop {
                    self.ws();
                    items.push(self.value(depth + 1)?);
                    self.ws();
                    if self.eat(b",") {
                        continue;
                    }
                    if self.eat(b"]") {
                        return Ok(Value::Array(items));
                    }
                    return Err(self.err("expected ',' or ']'"));
                }
            }
            Some(b'"') => Ok(Value::String(self.string()?)),
            Some(b't') if self.eat(b"true") => Ok(Value::Bool(true)),
            Some(b'f') if self.eat(b"false") => Ok(Value::Bool(false)),
            Some(b'n') if self.eat(b"null") => Ok(Value::Null),
            Some(c) if *c == b'-' || c.is_ascii_digit() => self.number(),
            Some(_) => Err(self.err("unexpected character")),
        }
    }

    fn number(&mut self) -> Result<Value, String> {
        let start = self.pos;
        while self.pos < self.s.len()
            && matches!(
                self.s[self.pos],
                b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9'
            )
        {
            self.pos += 1;
        }
        let text =
            std::str::from_utf8(&self.s[start..self.pos]).map_err(|_| self.err("bad number"))?;
        text.parse::<f64>()
            .map(Value::Number)
            .map_err(|_| self.err("bad number"))
    }

    fn hex4(&mut self) -> Result<u32, String> {
        let digits = self
            .s
            .get(self.pos..self.pos + 4)
            .ok_or_else(|| self.err("short \\u escape"))?;
        let text = std::str::from_utf8(digits).map_err(|_| self.err("bad \\u escape"))?;
        let v = u32::from_str_radix(text, 16).map_err(|_| self.err("bad \\u escape"))?;
        self.pos += 4;
        Ok(v)
    }

    fn string(&mut self) -> Result<String, String> {
        self.pos += 1;
        let mut out: Vec<u8> = Vec::new();
        loop {
            let Some(&c) = self.s.get(self.pos) else {
                return Err(self.err("unterminated string"));
            };
            self.pos += 1;
            match c {
                b'"' => break,
                b'\\' => {
                    let Some(&e) = self.s.get(self.pos) else {
                        return Err(self.err("unterminated escape"));
                    };
                    self.pos += 1;
                    let ch = match e {
                        b'"' => '"',
                        b'\\' => '\\',
                        b'/' => '/',
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'u' => {
                            let hi = self.hex4()?;
                            let code = if (0xD800..0xDC00).contains(&hi) {
                                if !self.eat(b"\\u") {
                                    return Err(self.err("unpaired surrogate"));
                                }
                                let lo = self.hex4()?;
                                if !(0xDC00..0xE000).contains(&lo) {
                                    return Err(self.err("unpaired surrogate"));
                                }
                                0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00)
                            } else {
                                hi
                            };
                            char::from_u32(code).ok_or_else(|| self.err("invalid code point"))?
                        }
                        _ => return Err(self.err("unknown escape")),
                    };
                    let mut buf = [0u8; 4];
                    out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                }
                c if c < 0x20 => return Err(self.err("control character in string")),
                c => out.push(c),
            }
        }
        String::from_utf8(out).map_err(|_| self.err("invalid UTF-8 in string"))
    }
}

/// Quotes and escapes `s` as a JSON string literal.
pub fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nested_documents() {
        let v = parse(r#" [ {"a": "x\"y\u00e9\ud83d\ude00", "b": [1, -2.5e1, true, null]} ] "#)
            .unwrap();
        let obj = &v.as_array().unwrap()[0];
        assert_eq!(
            obj.get("a").unwrap().as_str().unwrap(),
            "x\"y\u{e9}\u{1F600}"
        );
        let b = obj.get("b").unwrap().as_array().unwrap();
        assert_eq!(b[1].as_f64(), Some(-25.0));
        assert_eq!(b[2], Value::Bool(true));
        assert_eq!(b[3], Value::Null);
    }

    #[test]
    fn rejects_malformed_input() {
        for bad in [
            "",
            "[",
            "{\"a\" 1}",
            "[1,]",
            "\"abc",
            "[1] x",
            "{\"a\":\"\\q\"}",
            "\"\\ud800\"",
        ] {
            assert!(parse(bad).is_err(), "accepted {bad:?}");
        }
        let deep = "[".repeat(1000);
        assert!(parse(&deep).is_err());
    }

    #[test]
    fn quote_round_trips() {
        let s = "a\"b\\c\nd\u{1}é";
        assert_eq!(parse(&quote(s)).unwrap().as_str().unwrap(), s);
    }
}
