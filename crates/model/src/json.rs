//! Minimal JSON value, writer and parser – enough for the document format
//! without pulling in serde. Object key order is preserved so saved files
//! diff nicely in git.

use std::fmt::Write as _;

#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct JsonError {
    pub message: String,
    /// Byte offset of the problem (parse errors) or 0.
    pub offset: usize,
}

impl std::fmt::Display for JsonError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.offset > 0 {
            write!(f, "{} (at byte {})", self.message, self.offset)
        } else {
            f.write_str(&self.message)
        }
    }
}

impl std::error::Error for JsonError {}

fn err<T>(message: impl Into<String>) -> Result<T, JsonError> {
    Err(JsonError { message: message.into(), offset: 0 })
}

// ------------------------------------------------------------ construction

impl From<f64> for Json {
    fn from(v: f64) -> Self {
        Json::Num(v)
    }
}

impl From<usize> for Json {
    fn from(v: usize) -> Self {
        Json::Num(v as f64)
    }
}

impl From<u64> for Json {
    fn from(v: u64) -> Self {
        Json::Num(v as f64)
    }
}

impl From<bool> for Json {
    fn from(v: bool) -> Self {
        Json::Bool(v)
    }
}

impl From<&str> for Json {
    fn from(v: &str) -> Self {
        Json::Str(v.to_string())
    }
}

impl From<String> for Json {
    fn from(v: String) -> Self {
        Json::Str(v)
    }
}

impl<T: Into<Json>> From<Vec<T>> for Json {
    fn from(v: Vec<T>) -> Self {
        Json::Arr(v.into_iter().map(Into::into).collect())
    }
}

impl From<[f64; 2]> for Json {
    fn from(v: [f64; 2]) -> Self {
        Json::Arr(v.iter().map(|x| Json::Num(*x)).collect())
    }
}

impl From<[f64; 3]> for Json {
    fn from(v: [f64; 3]) -> Self {
        Json::Arr(v.iter().map(|x| Json::Num(*x)).collect())
    }
}

/// Build an object: `obj([("a", 1.0.into()), ...])`.
pub fn obj<const N: usize>(fields: [(&str, Json); N]) -> Json {
    Json::Obj(fields.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

// ----------------------------------------------------------------- access

impl Json {
    pub fn get(&self, key: &str) -> Result<&Json, JsonError> {
        match self {
            Json::Obj(fields) => match fields.iter().find(|(k, _)| k == key) {
                Some((_, v)) => Ok(v),
                None => err(format!("missing field '{key}'")),
            },
            _ => err(format!("expected an object with field '{key}'")),
        }
    }

    pub fn get_opt(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(fields) => fields.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Result<f64, JsonError> {
        match self {
            Json::Num(v) => Ok(*v),
            other => err(format!("expected a number, found {}", other.kind())),
        }
    }

    pub fn as_usize(&self) -> Result<usize, JsonError> {
        let v = self.as_f64()?;
        if v >= 0.0 && v.fract() == 0.0 && v <= 9.0e15 {
            Ok(v as usize)
        } else {
            err(format!("expected a non-negative integer, found {v}"))
        }
    }

    pub fn as_bool(&self) -> Result<bool, JsonError> {
        match self {
            Json::Bool(v) => Ok(*v),
            other => err(format!("expected true/false, found {}", other.kind())),
        }
    }

    pub fn as_str(&self) -> Result<&str, JsonError> {
        match self {
            Json::Str(s) => Ok(s),
            other => err(format!("expected a string, found {}", other.kind())),
        }
    }

    pub fn as_arr(&self) -> Result<&[Json], JsonError> {
        match self {
            Json::Arr(a) => Ok(a),
            other => err(format!("expected an array, found {}", other.kind())),
        }
    }

    pub fn as_vec2(&self) -> Result<[f64; 2], JsonError> {
        match self.as_arr()? {
            [x, y] => Ok([x.as_f64()?, y.as_f64()?]),
            _ => err("expected [x, y]"),
        }
    }

    pub fn as_vec3(&self) -> Result<[f64; 3], JsonError> {
        match self.as_arr()? {
            [x, y, z] => Ok([x.as_f64()?, y.as_f64()?, z.as_f64()?]),
            _ => err("expected [x, y, z]"),
        }
    }

    fn kind(&self) -> &'static str {
        match self {
            Json::Null => "null",
            Json::Bool(_) => "a boolean",
            Json::Num(_) => "a number",
            Json::Str(_) => "a string",
            Json::Arr(_) => "an array",
            Json::Obj(_) => "an object",
        }
    }
}

// ----------------------------------------------------------------- writer

impl Json {
    /// Pretty-printed with two-space indentation. Short arrays of scalars
    /// stay on one line.
    pub fn to_pretty(&self) -> String {
        let mut out = String::new();
        self.write(&mut out, 0);
        out.push('\n');
        out
    }

    fn is_scalar(&self) -> bool {
        !matches!(self, Json::Arr(_) | Json::Obj(_))
    }

    fn write(&self, out: &mut String, indent: usize) {
        let pad = |out: &mut String, n: usize| out.extend(std::iter::repeat_n(' ', n * 2));
        match self {
            Json::Null => out.push_str("null"),
            Json::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Json::Num(v) => {
                if v.is_finite() {
                    // Rust prints the shortest representation that parses
                    // back to the same f64.
                    let _ = write!(out, "{v}");
                } else {
                    out.push_str("null");
                }
            }
            Json::Str(s) => write_string(out, s),
            Json::Arr(items) if items.is_empty() => out.push_str("[]"),
            Json::Arr(items) if items.len() <= 8 && items.iter().all(Json::is_scalar) => {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    item.write(out, indent);
                }
                out.push(']');
            }
            Json::Arr(items) => {
                out.push_str("[\n");
                for (i, item) in items.iter().enumerate() {
                    pad(out, indent + 1);
                    item.write(out, indent + 1);
                    if i + 1 < items.len() {
                        out.push(',');
                    }
                    out.push('\n');
                }
                pad(out, indent);
                out.push(']');
            }
            Json::Obj(fields) if fields.is_empty() => out.push_str("{}"),
            Json::Obj(fields) => {
                out.push_str("{\n");
                for (i, (k, v)) in fields.iter().enumerate() {
                    pad(out, indent + 1);
                    write_string(out, k);
                    out.push_str(": ");
                    v.write(out, indent + 1);
                    if i + 1 < fields.len() {
                        out.push(',');
                    }
                    out.push('\n');
                }
                pad(out, indent);
                out.push('}');
            }
        }
    }
}

fn write_string(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
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
}

// ----------------------------------------------------------------- parser

pub fn parse(src: &str) -> Result<Json, JsonError> {
    let mut p = Parser { s: src.as_bytes(), src, pos: 0, depth: 0 };
    p.ws();
    let v = p.value()?;
    p.ws();
    if p.pos != p.s.len() {
        return p.fail("trailing characters after the document");
    }
    Ok(v)
}

struct Parser<'a> {
    s: &'a [u8],
    src: &'a str,
    pos: usize,
    depth: usize,
}

impl Parser<'_> {
    fn fail<T>(&self, message: &str) -> Result<T, JsonError> {
        Err(JsonError { message: message.to_string(), offset: self.pos.max(1) })
    }

    fn ws(&mut self) {
        while self.pos < self.s.len() && matches!(self.s[self.pos], b' ' | b'\n' | b'\r' | b'\t') {
            self.pos += 1;
        }
    }

    fn eat(&mut self, c: u8) -> bool {
        if self.s.get(self.pos) == Some(&c) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn literal(&mut self, word: &str, v: Json) -> Result<Json, JsonError> {
        if self.s[self.pos..].starts_with(word.as_bytes()) {
            self.pos += word.len();
            Ok(v)
        } else {
            self.fail("invalid literal")
        }
    }

    fn value(&mut self) -> Result<Json, JsonError> {
        self.depth += 1;
        if self.depth > 256 {
            return self.fail("nesting too deep");
        }
        let v = match self.s.get(self.pos) {
            None => self.fail("unexpected end of input"),
            Some(b'n') => self.literal("null", Json::Null),
            Some(b't') => self.literal("true", Json::Bool(true)),
            Some(b'f') => self.literal("false", Json::Bool(false)),
            Some(b'"') => self.string().map(Json::Str),
            Some(b'[') => self.array(),
            Some(b'{') => self.object(),
            Some(c) if *c == b'-' || c.is_ascii_digit() => self.number(),
            Some(_) => self.fail("unexpected character"),
        };
        self.depth -= 1;
        v
    }

    fn number(&mut self) -> Result<Json, JsonError> {
        let start = self.pos;
        while self.pos < self.s.len()
            && matches!(self.s[self.pos], b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9')
        {
            self.pos += 1;
        }
        match self.src[start..self.pos].parse::<f64>() {
            Ok(v) if v.is_finite() => Ok(Json::Num(v)),
            _ => {
                self.pos = start;
                self.fail("invalid number")
            }
        }
    }

    fn hex4(&mut self) -> Result<u32, JsonError> {
        let Some(h) = self.src.get(self.pos..self.pos + 4) else { return self.fail("short \\u escape") };
        let v = u32::from_str_radix(h, 16).or_else(|_| self.fail("bad \\u escape"))?;
        self.pos += 4;
        Ok(v)
    }

    fn string(&mut self) -> Result<String, JsonError> {
        self.pos += 1; // opening quote
        let mut out = String::new();
        loop {
            let start = self.pos;
            while self.pos < self.s.len() && !matches!(self.s[self.pos], b'"' | b'\\') {
                if self.s[self.pos] < 0x20 {
                    return self.fail("control character in string");
                }
                self.pos += 1;
            }
            out.push_str(&self.src[start..self.pos]);
            match self.s.get(self.pos) {
                None => return self.fail("unterminated string"),
                Some(b'"') => {
                    self.pos += 1;
                    return Ok(out);
                }
                _ => {
                    self.pos += 1; // backslash
                    let Some(&e) = self.s.get(self.pos) else { return self.fail("unterminated escape") };
                    self.pos += 1;
                    match e {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let mut cp = self.hex4()?;
                            if (0xD800..0xDC00).contains(&cp) {
                                // surrogate pair
                                if !(self.eat(b'\\') && self.eat(b'u')) {
                                    return self.fail("lone surrogate");
                                }
                                let lo = self.hex4()?;
                                if !(0xDC00..0xE000).contains(&lo) {
                                    return self.fail("bad surrogate pair");
                                }
                                cp = 0x10000 + ((cp - 0xD800) << 10) + (lo - 0xDC00);
                            }
                            match char::from_u32(cp) {
                                Some(c) => out.push(c),
                                None => return self.fail("invalid code point"),
                            }
                        }
                        _ => return self.fail("unknown escape"),
                    }
                }
            }
        }
    }

    fn array(&mut self) -> Result<Json, JsonError> {
        self.pos += 1;
        let mut items = Vec::new();
        self.ws();
        if self.eat(b']') {
            return Ok(Json::Arr(items));
        }
        loop {
            self.ws();
            items.push(self.value()?);
            self.ws();
            if self.eat(b']') {
                return Ok(Json::Arr(items));
            }
            if !self.eat(b',') {
                return self.fail("expected ',' or ']'");
            }
        }
    }

    fn object(&mut self) -> Result<Json, JsonError> {
        self.pos += 1;
        let mut fields = Vec::new();
        self.ws();
        if self.eat(b'}') {
            return Ok(Json::Obj(fields));
        }
        loop {
            self.ws();
            if self.s.get(self.pos) != Some(&b'"') {
                return self.fail("expected a field name");
            }
            let key = self.string()?;
            self.ws();
            if !self.eat(b':') {
                return self.fail("expected ':'");
            }
            self.ws();
            let v = self.value()?;
            fields.push((key, v));
            self.ws();
            if self.eat(b'}') {
                return Ok(Json::Obj(fields));
            }
            if !self.eat(b',') {
                return self.fail("expected ',' or '}'");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_values() {
        let v = obj([
            ("name", "Plätte \"A\"\n\t\\".into()),
            ("n", 0.1.into()),
            ("tiny", 1e-300.into()),
            ("neg", (-12.5).into()),
            ("flags", vec![true, false].into()),
            ("nested", Json::Arr(vec![obj([("x", Json::Null)]), Json::Arr(vec![])])),
            ("empty", Json::Obj(vec![])),
        ]);
        let text = v.to_pretty();
        assert_eq!(parse(&text).unwrap(), v);
    }

    #[test]
    fn floats_roundtrip_exactly() {
        for x in [0.1 + 0.2, std::f64::consts::PI, 1.0 / 3.0, 123456789.123456789, -0.0] {
            let back = parse(&Json::Num(x).to_pretty()).unwrap().as_f64().unwrap();
            assert_eq!(back.to_bits(), x.to_bits());
        }
    }

    #[test]
    fn parses_standard_json() {
        let v = parse(r#" {"a": [1, 2.5e3, -3], "b": "ä😀", "c": null} "#).unwrap();
        assert_eq!(v.get("a").unwrap().as_arr().unwrap()[1].as_f64().unwrap(), 2500.0);
        assert_eq!(v.get("b").unwrap().as_str().unwrap(), "ä😀");
    }

    #[test]
    fn rejects_garbage() {
        for bad in ["", "{", "[1,]", "{\"a\" 1}", "tru", "\"abc", "1 2", "[01x]", "{\"a\":1,}"] {
            assert!(parse(bad).is_err(), "should reject {bad:?}");
        }
        let deep = "[".repeat(1000);
        assert!(parse(&deep).is_err());
    }

    #[test]
    fn typed_access_errors_are_readable() {
        let v = parse(r#"{"a": "x"}"#).unwrap();
        assert_eq!(v.get("b").unwrap_err().message, "missing field 'b'");
        assert!(v.get("a").unwrap().as_f64().unwrap_err().message.contains("string"));
        assert!(Json::Num(1.5).as_usize().is_err());
    }
}
