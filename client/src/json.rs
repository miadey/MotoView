//! A tiny, dependency-free JSON parser — just enough to read MotoView render
//! batches (the only JSON the client ever consumes). Produces a borrowed-free
//! `Value` tree. Not a general-purpose library; it covers objects, arrays,
//! strings (with escapes), numbers, booleans and null.

use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Value>),
    Obj(BTreeMap<String, Value>),
}

impl Value {
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Obj(m) => m.get(key),
            _ => None,
        }
    }
    pub fn as_str(&self) -> &str {
        match self {
            Value::Str(s) => s,
            _ => "",
        }
    }
    pub fn as_array(&self) -> &[Value] {
        match self {
            Value::Arr(a) => a,
            _ => &[],
        }
    }
    /// Convenience: read a string field, or "" if missing.
    pub fn str_field(&self, key: &str) -> String {
        self.get(key).map(|v| v.as_str().to_string()).unwrap_or_default()
    }
}

pub fn parse(input: &str) -> Option<Value> {
    let bytes = input.as_bytes();
    let mut p = Parser { b: bytes, i: 0 };
    p.skip_ws();
    let v = p.value()?;
    Some(v)
}

struct Parser<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> u8 {
        if self.i < self.b.len() {
            self.b[self.i]
        } else {
            0
        }
    }
    fn skip_ws(&mut self) {
        while self.i < self.b.len() {
            match self.b[self.i] {
                b' ' | b'\t' | b'\n' | b'\r' => self.i += 1,
                _ => break,
            }
        }
    }
    fn value(&mut self) -> Option<Value> {
        self.skip_ws();
        match self.peek() {
            b'{' => self.object(),
            b'[' => self.array(),
            b'"' => Some(Value::Str(self.string()?)),
            b't' | b'f' => self.boolean(),
            b'n' => self.null(),
            _ => self.number(),
        }
    }
    fn object(&mut self) -> Option<Value> {
        self.i += 1; // {
        let mut m = BTreeMap::new();
        self.skip_ws();
        if self.peek() == b'}' {
            self.i += 1;
            return Some(Value::Obj(m));
        }
        loop {
            self.skip_ws();
            let key = self.string()?;
            self.skip_ws();
            if self.peek() != b':' {
                return None;
            }
            self.i += 1;
            let val = self.value()?;
            m.insert(key, val);
            self.skip_ws();
            match self.peek() {
                b',' => self.i += 1,
                b'}' => {
                    self.i += 1;
                    return Some(Value::Obj(m));
                }
                _ => return None,
            }
        }
    }
    fn array(&mut self) -> Option<Value> {
        self.i += 1; // [
        let mut a = Vec::new();
        self.skip_ws();
        if self.peek() == b']' {
            self.i += 1;
            return Some(Value::Arr(a));
        }
        loop {
            let v = self.value()?;
            a.push(v);
            self.skip_ws();
            match self.peek() {
                b',' => self.i += 1,
                b']' => {
                    self.i += 1;
                    return Some(Value::Arr(a));
                }
                _ => return None,
            }
        }
    }
    fn string(&mut self) -> Option<String> {
        if self.peek() != b'"' {
            return None;
        }
        self.i += 1;
        // Collect raw bytes so multi-byte UTF-8 in the HTML survives intact.
        let mut out: Vec<u8> = Vec::new();
        while self.i < self.b.len() {
            let c = self.b[self.i];
            self.i += 1;
            match c {
                b'"' => return Some(String::from_utf8_lossy(&out).into_owned()),
                b'\\' => {
                    let e = self.b.get(self.i).copied().unwrap_or(0);
                    self.i += 1;
                    match e {
                        b'"' => out.push(b'"'),
                        b'\\' => out.push(b'\\'),
                        b'/' => out.push(b'/'),
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'b' => out.push(0x08),
                        b'f' => out.push(0x0C),
                        b'u' => {
                            let end = (self.i + 4).min(self.b.len());
                            let hex = std::str::from_utf8(&self.b[self.i..end]).ok()?;
                            let code = u32::from_str_radix(hex, 16).ok()?;
                            self.i = end; // advance by the bytes actually consumed
                            if let Some(ch) = char::from_u32(code) {
                                let mut buf = [0u8; 4];
                                out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                            }
                        }
                        _ => {}
                    }
                }
                _ => out.push(c),
            }
        }
        None
    }
    fn boolean(&mut self) -> Option<Value> {
        if self.b[self.i..].starts_with(b"true") {
            self.i += 4;
            Some(Value::Bool(true))
        } else if self.b[self.i..].starts_with(b"false") {
            self.i += 5;
            Some(Value::Bool(false))
        } else {
            None
        }
    }
    fn null(&mut self) -> Option<Value> {
        if self.b[self.i..].starts_with(b"null") {
            self.i += 4;
            Some(Value::Null)
        } else {
            None
        }
    }
    fn number(&mut self) -> Option<Value> {
        let start = self.i;
        while self.i < self.b.len() {
            match self.b[self.i] {
                b'0'..=b'9' | b'-' | b'+' | b'.' | b'e' | b'E' => self.i += 1,
                _ => break,
            }
        }
        let s = std::str::from_utf8(&self.b[start..self.i]).ok()?;
        s.parse::<f64>().ok().map(Value::Num)
    }
}
