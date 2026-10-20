use serde_json::{Map, Number, Value};

pub const MAX_DEPTH: usize = 64;

pub struct Limits {
    pub depth: usize,
    pub values: usize,
}

impl Limits {
    pub fn for_bytes(n: usize) -> Limits {
        Limits { depth: MAX_DEPTH, values: 1_000_000 + n / 8 }
    }
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
    left: usize,
    depth: usize,
}

pub fn parse(bytes: &[u8], limits: &Limits) -> Result<Value, String> {
    let text = std::str::from_utf8(bytes).map_err(|e| format!("not UTF-8: {e}"))?;
    let mut p = Parser { s: text.as_bytes(), i: 0, left: limits.values, depth: limits.depth };
    p.ws();
    let v = p.value()?;
    p.ws();
    if p.i != p.s.len() {
        return Err(p.err("trailing characters"));
    }
    Ok(v)
}

pub fn text(text: &str) -> Result<Value, String> {
    parse(text.as_bytes(), &Limits::for_bytes(text.len()))
}

impl Parser<'_> {
    fn err(&self, what: &str) -> String {
        format!("{what} at byte {}", self.i)
    }

    fn ws(&mut self) {
        while self.i < self.s.len() && matches!(self.s[self.i], b' ' | b'\t' | b'\n' | b'\r') {
            self.i += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }

    fn word(&mut self, w: &[u8], v: Value) -> Result<Value, String> {
        if self.s[self.i..].starts_with(w) {
            self.i += w.len();
            Ok(v)
        } else {
            Err(self.err("unexpected input"))
        }
    }

    fn value(&mut self) -> Result<Value, String> {
        if self.left == 0 {
            return Err(self.err("more values than the limit"));
        }
        self.left -= 1;
        match self.peek() {
            None => Err(self.err("unexpected end")),
            Some(b'{') => self.nested(true),
            Some(b'[') => self.nested(false),
            Some(b'"') => self.string().map(Value::String),
            Some(b't') => self.word(b"true", Value::Bool(true)),
            Some(b'f') => self.word(b"false", Value::Bool(false)),
            Some(b'n') => self.word(b"null", Value::Null),
            Some(b'-' | b'0'..=b'9') => self.number(),
            Some(_) => Err(self.err("unexpected character")),
        }
    }

    fn nested(&mut self, object: bool) -> Result<Value, String> {
        if self.depth == 0 {
            return Err(self.err("nested deeper than the limit"));
        }
        self.depth -= 1;
        self.i += 1;
        let close = if object { b'}' } else { b']' };
        let mut map = Map::new();
        let mut list = Vec::new();
        self.ws();
        if self.peek() == Some(close) {
            self.i += 1;
        } else {
            loop {
                self.ws();
                if object {
                    if self.peek() != Some(b'"') {
                        return Err(self.err("expected a key"));
                    }
                    let k = self.string()?;
                    self.ws();
                    if self.peek() != Some(b':') {
                        return Err(self.err("expected ':'"));
                    }
                    self.i += 1;
                    self.ws();
                    let v = self.value()?;
                    map.insert(k, v);
                } else {
                    list.push(self.value()?);
                }
                self.ws();
                match self.peek() {
                    Some(b',') => self.i += 1,
                    Some(c) if c == close => {
                        self.i += 1;
                        break;
                    }
                    _ => return Err(self.err("expected ',' or a closing bracket")),
                }
            }
        }
        self.depth += 1;
        Ok(if object { Value::Object(map) } else { Value::Array(list) })
    }

    fn hex4(&mut self) -> Result<u32, String> {
        let s = self.s;
        let h = s.get(self.i..self.i + 4).filter(|h| h.iter().all(u8::is_ascii_hexdigit)).ok_or_else(|| self.err("invalid \\u escape"))?;
        self.i += 4;
        Ok(h.iter().fold(0u32, |a, &b| a * 16 + (b as char).to_digit(16).unwrap_or(0)))
    }

    fn string(&mut self) -> Result<String, String> {
        let s = self.s;
        self.i += 1;
        let mut out = String::new();
        loop {
            let start = self.i;
            while self.i < s.len() && !matches!(s[self.i], b'"' | b'\\') && s[self.i] >= 0x20 {
                self.i += 1;
            }
            out.push_str(std::str::from_utf8(&s[start..self.i]).map_err(|_| self.err("invalid UTF-8"))?);
            match self.peek() {
                Some(b'"') => {
                    self.i += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.i += 1;
                    let c = self.peek().ok_or_else(|| self.err("unexpected end"))?;
                    self.i += 1;
                    match c {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let hi = self.hex4()?;
                            let code = if (0xD800..0xDC00).contains(&hi) {
                                if !s[self.i..].starts_with(b"\\u") {
                                    return Err(self.err("lone surrogate"));
                                }
                                self.i += 2;
                                let lo = self.hex4()?;
                                if !(0xDC00..0xE000).contains(&lo) {
                                    return Err(self.err("invalid surrogate pair"));
                                }
                                0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00)
                            } else if (0xDC00..0xE000).contains(&hi) {
                                return Err(self.err("lone surrogate"));
                            } else {
                                hi
                            };
                            out.push(char::from_u32(code).ok_or_else(|| self.err("invalid \\u escape"))?);
                        }
                        _ => return Err(self.err("invalid escape")),
                    }
                }
                Some(_) => return Err(self.err("control character in a string")),
                None => return Err(self.err("unterminated string")),
            }
        }
    }

    fn digits(&mut self) -> usize {
        let start = self.i;
        while self.i < self.s.len() && self.s[self.i].is_ascii_digit() {
            self.i += 1;
        }
        self.i - start
    }

    fn number(&mut self) -> Result<Value, String> {
        let s = self.s;
        let start = self.i;
        if s[self.i] == b'-' {
            self.i += 1;
        }
        let int_start = self.i;
        let n = self.digits();
        if n == 0 || (s[int_start] == b'0' && n > 1) {
            return Err(self.err("invalid number"));
        }
        let mut float = false;
        if self.peek() == Some(b'.') {
            self.i += 1;
            float = true;
            if self.digits() == 0 {
                return Err(self.err("invalid number"));
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.i += 1;
            float = true;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.i += 1;
            }
            if self.digits() == 0 {
                return Err(self.err("invalid number"));
            }
        }
        let text = std::str::from_utf8(&s[start..self.i]).map_err(|_| self.err("invalid number"))?;
        if !float {
            if let Ok(u) = text.parse::<u64>() {
                return Ok(Value::Number(u.into()));
            }
            if let Ok(i) = text.parse::<i64>()
                && i != 0
            {
                return Ok(Value::Number(i.into()));
            }
        }
        let f: f64 = text.parse().map_err(|_| self.err("invalid number"))?;
        Number::from_f64(f).filter(|_| f.is_finite()).map(Value::Number).ok_or_else(|| self.err("number out of range"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(t: &str) -> Result<Value, String> {
        parse(t.as_bytes(), &Limits::for_bytes(t.len()))
    }

    #[test]
    fn floats_round_trip_exactly_and_values_match_serde() {
        for t in ["0.10937899999999999", "0.109379", "1e-7", "-0.0", "123456789.123456789", "5e-324", "1.7976931348623157e308"] {
            let v = p(t).unwrap();
            let exact: f64 = t.parse().unwrap();
            assert_eq!(v.as_f64().unwrap().to_bits(), exact.to_bits(), "{t}");
            assert_eq!(p(&serde_json::to_string(&v).unwrap()).unwrap(), v, "{t}");
        }
        let doc = r#"{"a": [1, -2, 18446744073709551615, 18446744073709551616, -9223372036854775809, -0, true, false, null],
                      "b": {"c": "x\u00e9\ud83d\ude00\n\"\\/", "d": 2.5e3}, "e": []}"#;
        assert_eq!(p(doc).unwrap(), serde_json::from_str::<Value>(doc).unwrap());
    }

    #[test]
    fn malformed_deep_and_oversized_input_is_refused() {
        for bad in ["", "{", "[1,]", "{\"a\":1,}", "01", "1.", "-", "\"\\ud800\"", "\"a\nb\"", "nul", "1 2", "1e999", "{\"a\" 1}"] {
            assert!(p(bad).is_err(), "{bad:?}");
        }
        let deep = "[".repeat(MAX_DEPTH + 1) + &"]".repeat(MAX_DEPTH + 1);
        assert!(p(&deep).unwrap_err().contains("deeper"));
        let ok = "[".repeat(MAX_DEPTH) + &"]".repeat(MAX_DEPTH);
        assert!(p(&ok).is_ok());
        let many = format!("[{}0]", "0,".repeat(100));
        assert!(parse(many.as_bytes(), &Limits { depth: 4, values: 50 }).unwrap_err().contains("limit"));
    }
}
