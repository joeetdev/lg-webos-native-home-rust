//! A small JSON implementation tailored to the device state files and LS2
//! responses.
//!
//! Why not a general-purpose crate? The safety rules need properties that
//! general parsers do not provide together:
//!
//! * duplicate object keys are **rejected** (a later duplicate could otherwise
//!   silently override an ownership field);
//! * objects keep insertion order for faithful round-trips of foreign
//!   `keyFilters` entries, while equality ignores key order like a dictionary;
//! * numbers keep their source text, so foreign values are written back
//!   unchanged, and integers are distinguishable from floats;
//! * nesting depth is bounded, so hostile input cannot exhaust the small stack.
//!
//! Serialisation reproduces Python's `json.dumps` layouts byte for byte
//! (`ensure_ascii`, the compact/default/indented separators and `sort_keys`),
//! which keeps content-addressed names and state files identical to the
//! installations created by the previous implementation.

use std::fmt;

/// Maximum nesting depth accepted by the parser.
pub const MAX_DEPTH: usize = 128;

/// A JSON value.
#[derive(Clone, Debug)]
pub enum Value {
    /// `null`
    Null,
    /// `true` / `false`
    Bool(bool),
    /// Any JSON number, kept as validated source text.
    Number(Number),
    /// A string.
    String(String),
    /// An array.
    Array(Vec<Value>),
    /// An object with unique keys in insertion order.
    Object(Map),
}

/// A JSON number stored as its (validated) textual form.
#[derive(Clone, Debug)]
pub struct Number {
    text: Box<str>,
    integer: bool,
}

/// An insertion-ordered object with unique keys.
///
/// Lookups are linear: every object handled by this crate has a handful of
/// keys, and a vector is both smaller and faster than a hash map at that size.
#[derive(Clone, Debug, Default)]
pub struct Map {
    entries: Vec<(String, Value)>,
}

/// Why parsing failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParseError {
    /// The input bytes are not valid UTF-8.
    Utf8,
    /// The input is not valid JSON.
    Syntax {
        /// Byte offset of the problem.
        offset: usize,
        /// What was expected.
        message: &'static str,
    },
    /// An object repeats a key.
    DuplicateKey(String),
    /// Nesting exceeds [`MAX_DEPTH`].
    TooDeep,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::Utf8 => f.write_str("JSON input is not valid UTF-8"),
            ParseError::Syntax { offset, message } => {
                write!(f, "Invalid JSON at byte {offset}: {message}")
            }
            ParseError::DuplicateKey(key) => write!(f, "Duplicate JSON key: {key}"),
            ParseError::TooDeep => f.write_str("JSON nesting is too deep"),
        }
    }
}

impl std::error::Error for ParseError {}

impl Number {
    fn from_text(text: &str, integer: bool) -> Self {
        Number {
            text: text.into(),
            integer,
        }
    }

    /// True when the number has neither a fraction nor an exponent.
    pub fn is_integer(&self) -> bool {
        self.integer
    }

    /// The value as `i64`, if it is an integer in range.
    pub fn as_i64(&self) -> Option<i64> {
        if self.integer {
            self.text.parse().ok()
        } else {
            None
        }
    }

    /// The value as `u64`, if it is a non-negative integer in range.
    pub fn as_u64(&self) -> Option<u64> {
        if self.integer {
            self.normalized_integer().parse().ok()
        } else {
            None
        }
    }

    /// The value as `f64` (always finite: non-finite input is rejected).
    pub fn as_f64(&self) -> f64 {
        self.text.parse().unwrap_or(f64::NAN)
    }

    /// Integer text without a negative sign on zero (`-0` is `0`).
    fn normalized_integer(&self) -> &str {
        match self.text.strip_prefix('-') {
            Some(digits) if digits.bytes().all(|b| b == b'0') => digits,
            _ => &self.text,
        }
    }

    /// Strict equality: integers only equal integers, floats compare by bit
    /// pattern. Equivalent to comparing Python `json.dumps` output.
    fn strict_eq(&self, other: &Number) -> bool {
        match (self.integer, other.integer) {
            (true, true) => self.normalized_integer() == other.normalized_integer(),
            (false, false) => self.as_f64().to_bits() == other.as_f64().to_bits(),
            _ => false,
        }
    }
}

impl PartialEq for Number {
    /// Numeric equality across integer and float forms (`1 == 1.0`), the
    /// semantics of a Python `==` comparison on decoded JSON.
    fn eq(&self, other: &Number) -> bool {
        if self.integer && other.integer {
            self.normalized_integer() == other.normalized_integer()
        } else {
            self.as_f64() == other.as_f64()
        }
    }
}

impl From<u64> for Number {
    fn from(value: u64) -> Self {
        Number::from_text(&value.to_string(), true)
    }
}

impl From<i64> for Number {
    fn from(value: i64) -> Self {
        Number::from_text(&value.to_string(), true)
    }
}

impl Map {
    /// An empty object.
    pub fn new() -> Self {
        Map {
            entries: Vec::new(),
        }
    }

    /// Number of keys.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when there are no keys.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Looks up a key.
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.entries.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    /// Looks up a key for mutation.
    pub fn get_mut(&mut self, key: &str) -> Option<&mut Value> {
        self.entries
            .iter_mut()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v)
    }

    /// True when the key is present (even with a `null` value).
    pub fn contains_key(&self, key: &str) -> bool {
        self.entries.iter().any(|(k, _)| k == key)
    }

    /// Sets a key, keeping its position when it already exists.
    pub fn insert(&mut self, key: impl Into<String>, value: impl Into<Value>) {
        let key = key.into();
        let value = value.into();
        match self.get_mut(&key) {
            Some(slot) => *slot = value,
            None => self.entries.push((key, value)),
        }
    }

    /// Removes a key, returning its value.
    pub fn remove(&mut self, key: &str) -> Option<Value> {
        let index = self.entries.iter().position(|(k, _)| k == key)?;
        Some(self.entries.remove(index).1)
    }

    /// Iterates over keys in insertion order.
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(|(k, _)| k.as_str())
    }

    /// Iterates over entries in insertion order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &Value)> {
        self.entries.iter().map(|(k, v)| (k.as_str(), v))
    }

    /// True when the key set is exactly `expected` (order-insensitive).
    pub fn has_exact_keys(&self, expected: &[&str]) -> bool {
        self.len() == expected.len() && expected.iter().all(|key| self.contains_key(key))
    }

    fn strict_eq(&self, other: &Map) -> bool {
        self.len() == other.len()
            && self
                .iter()
                .all(|(k, v)| other.get(k).is_some_and(|o| v.strict_eq(o)))
    }
}

impl PartialEq for Map {
    /// Dictionary equality: same keys and equal values, regardless of order.
    fn eq(&self, other: &Map) -> bool {
        self.len() == other.len() && self.iter().all(|(k, v)| other.get(k) == Some(v))
    }
}

impl<K: Into<String>, V: Into<Value>> FromIterator<(K, V)> for Map {
    fn from_iter<I: IntoIterator<Item = (K, V)>>(iter: I) -> Self {
        let mut map = Map::new();
        for (key, value) in iter {
            map.insert(key, value);
        }
        map
    }
}

impl Value {
    /// The string, if this is a string.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s),
            _ => None,
        }
    }

    /// The boolean, if this is a boolean.
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// The number, if this is a number.
    pub fn as_number(&self) -> Option<&Number> {
        match self {
            Value::Number(n) => Some(n),
            _ => None,
        }
    }

    /// The value as `i64`, if it is an integer number in range.
    pub fn as_i64(&self) -> Option<i64> {
        self.as_number().and_then(Number::as_i64)
    }

    /// The value as `u64`, if it is a non-negative integer number in range.
    pub fn as_u64(&self) -> Option<u64> {
        self.as_number().and_then(Number::as_u64)
    }

    /// True for integer numbers (Python `type(x) is int`; booleans excluded).
    pub fn is_integer(&self) -> bool {
        matches!(self, Value::Number(n) if n.is_integer())
    }

    /// The elements, if this is an array.
    pub fn as_array(&self) -> Option<&Vec<Value>> {
        match self {
            Value::Array(a) => Some(a),
            _ => None,
        }
    }

    /// The object, if this is an object.
    pub fn as_object(&self) -> Option<&Map> {
        match self {
            Value::Object(o) => Some(o),
            _ => None,
        }
    }

    /// The object for mutation, if this is an object.
    pub fn as_object_mut(&mut self) -> Option<&mut Map> {
        match self {
            Value::Object(o) => Some(o),
            _ => None,
        }
    }

    /// Looks up a key when this is an object.
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.as_object().and_then(|o| o.get(key))
    }

    /// True for `null`.
    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    /// Type- and order-sensitive equality, equivalent to comparing
    /// `json.dumps(value, sort_keys=True)` output: `0`, `0.0` and `false` all
    /// differ, object key order does not matter, array order does.
    pub fn strict_eq(&self, other: &Value) -> bool {
        match (self, other) {
            (Value::Null, Value::Null) => true,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Number(a), Value::Number(b)) => a.strict_eq(b),
            (Value::String(a), Value::String(b)) => a == b,
            (Value::Array(a), Value::Array(b)) => {
                a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.strict_eq(y))
            }
            (Value::Object(a), Value::Object(b)) => a.strict_eq(b),
            _ => false,
        }
    }
}

impl PartialEq for Value {
    /// Python `==` semantics on decoded JSON, except that booleans never equal
    /// numbers (`true != 1`), which only makes comparisons stricter.
    fn eq(&self, other: &Value) -> bool {
        match (self, other) {
            (Value::Null, Value::Null) => true,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Number(a), Value::Number(b)) => a == b,
            (Value::String(a), Value::String(b)) => a == b,
            (Value::Array(a), Value::Array(b)) => a == b,
            (Value::Object(a), Value::Object(b)) => a == b,
            _ => false,
        }
    }
}

impl From<&str> for Value {
    fn from(value: &str) -> Self {
        Value::String(value.to_owned())
    }
}

impl From<String> for Value {
    fn from(value: String) -> Self {
        Value::String(value)
    }
}

impl From<&String> for Value {
    fn from(value: &String) -> Self {
        Value::String(value.clone())
    }
}

impl From<bool> for Value {
    fn from(value: bool) -> Self {
        Value::Bool(value)
    }
}

impl From<u64> for Value {
    fn from(value: u64) -> Self {
        Value::Number(Number::from(value))
    }
}

impl From<u32> for Value {
    fn from(value: u32) -> Self {
        Value::Number(Number::from(u64::from(value)))
    }
}

impl From<i64> for Value {
    fn from(value: i64) -> Self {
        Value::Number(Number::from(value))
    }
}

impl From<usize> for Value {
    fn from(value: usize) -> Self {
        Value::Number(Number::from(value as u64))
    }
}

impl From<Map> for Value {
    fn from(value: Map) -> Self {
        Value::Object(value)
    }
}

impl From<Vec<Value>> for Value {
    fn from(value: Vec<Value>) -> Self {
        Value::Array(value)
    }
}

impl<T: Into<Value>> From<Option<T>> for Value {
    fn from(value: Option<T>) -> Self {
        value.map_or(Value::Null, Into::into)
    }
}

/// Builds an object from `key => value` pairs, preserving their order.
#[macro_export]
macro_rules! json_object {
    ($($key:expr => $value:expr),* $(,)?) => {{
        #[allow(unused_mut)]
        let mut map = $crate::json::Map::new();
        $(map.insert($key, $crate::json::Value::from($value));)*
        $crate::json::Value::Object(map)
    }};
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

/// Parses UTF-8 bytes as one JSON document.
pub fn parse_bytes(bytes: &[u8]) -> Result<Value, ParseError> {
    let text = std::str::from_utf8(bytes).map_err(|_| ParseError::Utf8)?;
    parse(text)
}

/// Parses one JSON document. Surrounding JSON whitespace is allowed.
pub fn parse(text: &str) -> Result<Value, ParseError> {
    let mut parser = Parser {
        bytes: text.as_bytes(),
        text,
        pos: 0,
    };
    parser.skip_whitespace();
    let value = parser.value(0)?;
    parser.skip_whitespace();
    if parser.pos != parser.bytes.len() {
        return Err(parser.error("extra data after the document"));
    }
    Ok(value)
}

struct Parser<'a> {
    bytes: &'a [u8],
    text: &'a str,
    pos: usize,
}

impl<'a> Parser<'a> {
    fn error(&self, message: &'static str) -> ParseError {
        ParseError::Syntax {
            offset: self.pos,
            message,
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn skip_whitespace(&mut self) {
        while let Some(b' ' | b'\t' | b'\n' | b'\r') = self.peek() {
            self.pos += 1;
        }
    }

    fn literal(&mut self, word: &'static str, value: Value) -> Result<Value, ParseError> {
        if self.bytes[self.pos..].starts_with(word.as_bytes()) {
            self.pos += word.len();
            Ok(value)
        } else {
            Err(self.error("invalid literal"))
        }
    }

    fn value(&mut self, depth: usize) -> Result<Value, ParseError> {
        match self.peek() {
            Some(b'{') => self.object(depth + 1),
            Some(b'[') => self.array(depth + 1),
            Some(b'"') => self.string().map(Value::String),
            Some(b't') => self.literal("true", Value::Bool(true)),
            Some(b'f') => self.literal("false", Value::Bool(false)),
            Some(b'n') => self.literal("null", Value::Null),
            Some(b'-' | b'0'..=b'9') => self.number(),
            Some(_) => Err(self.error("expected a value")),
            None => Err(self.error("unexpected end of input")),
        }
    }

    fn object(&mut self, depth: usize) -> Result<Value, ParseError> {
        if depth > MAX_DEPTH {
            return Err(ParseError::TooDeep);
        }
        self.pos += 1;
        let mut map = Map::new();
        self.skip_whitespace();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(Value::Object(map));
        }
        loop {
            self.skip_whitespace();
            if self.peek() != Some(b'"') {
                return Err(self.error("expected an object key"));
            }
            let key = self.string()?;
            self.skip_whitespace();
            if self.peek() != Some(b':') {
                return Err(self.error("expected ':'"));
            }
            self.pos += 1;
            self.skip_whitespace();
            let value = self.value(depth)?;
            if map.contains_key(&key) {
                return Err(ParseError::DuplicateKey(key));
            }
            map.entries.push((key, value));
            self.skip_whitespace();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(Value::Object(map));
                }
                _ => return Err(self.error("expected ',' or '}'")),
            }
        }
    }

    fn array(&mut self, depth: usize) -> Result<Value, ParseError> {
        if depth > MAX_DEPTH {
            return Err(ParseError::TooDeep);
        }
        self.pos += 1;
        let mut items = Vec::new();
        self.skip_whitespace();
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Ok(Value::Array(items));
        }
        loop {
            self.skip_whitespace();
            items.push(self.value(depth)?);
            self.skip_whitespace();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b']') => {
                    self.pos += 1;
                    return Ok(Value::Array(items));
                }
                _ => return Err(self.error("expected ',' or ']'")),
            }
        }
    }

    fn digits(&mut self) -> usize {
        let start = self.pos;
        while let Some(b'0'..=b'9') = self.peek() {
            self.pos += 1;
        }
        self.pos - start
    }

    fn number(&mut self) -> Result<Value, ParseError> {
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        match self.peek() {
            Some(b'0') => self.pos += 1,
            Some(b'1'..=b'9') => {
                self.digits();
            }
            _ => return Err(self.error("invalid number")),
        }
        let mut integer = true;
        if self.peek() == Some(b'.') {
            self.pos += 1;
            if self.digits() == 0 {
                return Err(self.error("invalid number fraction"));
            }
            integer = false;
        }
        if let Some(b'e' | b'E') = self.peek() {
            self.pos += 1;
            if let Some(b'+' | b'-') = self.peek() {
                self.pos += 1;
            }
            if self.digits() == 0 {
                return Err(self.error("invalid number exponent"));
            }
            integer = false;
        }
        let text = &self.text[start..self.pos];
        let number = Number::from_text(text, integer);
        if !integer && !number.as_f64().is_finite() {
            return Err(self.error("number out of range"));
        }
        Ok(Value::Number(number))
    }

    fn hex4(&mut self) -> Result<u32, ParseError> {
        let digits = self
            .bytes
            .get(self.pos..self.pos + 4)
            .ok_or(self.error("short \\u escape"))?;
        let mut value = 0;
        for &digit in digits {
            let nibble = (digit as char)
                .to_digit(16)
                .ok_or(self.error("invalid \\u escape"))?;
            value = value * 16 + nibble;
        }
        self.pos += 4;
        Ok(value)
    }

    fn string(&mut self) -> Result<String, ParseError> {
        self.pos += 1;
        let mut out = String::new();
        loop {
            let run_start = self.pos;
            while let Some(byte) = self.peek() {
                if byte == b'"' || byte == b'\\' || byte < 0x20 {
                    break;
                }
                self.pos += 1;
            }
            out.push_str(&self.text[run_start..self.pos]);
            match self.peek() {
                Some(b'"') => {
                    self.pos += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.pos += 1;
                    let escape = self.peek().ok_or(self.error("unterminated escape"))?;
                    self.pos += 1;
                    match escape {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => out.push(self.unicode_escape()?),
                        _ => return Err(self.error("invalid escape")),
                    }
                }
                Some(_) => return Err(self.error("control character in string")),
                None => return Err(self.error("unterminated string")),
            }
        }
    }

    fn unicode_escape(&mut self) -> Result<char, ParseError> {
        let first = self.hex4()?;
        let code = if (0xD800..0xDC00).contains(&first) {
            if !self.bytes[self.pos..].starts_with(b"\\u") {
                return Err(self.error("unpaired surrogate"));
            }
            self.pos += 2;
            let second = self.hex4()?;
            if !(0xDC00..0xE000).contains(&second) {
                return Err(self.error("unpaired surrogate"));
            }
            0x10000 + ((first - 0xD800) << 10) + (second - 0xDC00)
        } else {
            first
        };
        char::from_u32(code).ok_or(self.error("unpaired surrogate"))
    }
}

// ---------------------------------------------------------------------------
// Serialisation (byte-compatible with Python's json.dumps)
// ---------------------------------------------------------------------------

/// Output layout, named after the equivalent `json.dumps` arguments.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    /// `separators=(',', ':')`
    Compact,
    /// Python's default `separators=(', ', ': ')` on one line.
    Spaced,
    /// `indent=2`
    Indented,
}

/// Serialises `value` like `json.dumps(value, ensure_ascii=True, ...)`.
pub fn to_string(value: &Value, style: Style, sort_keys: bool) -> String {
    let mut out = String::new();
    write_value(&mut out, value, style, sort_keys, 0);
    out
}

/// Compact form with insertion order: `json.dumps(v, separators=(',', ':'))`.
pub fn compact(value: &Value) -> String {
    to_string(value, Style::Compact, false)
}

fn newline_indent(out: &mut String, level: usize) {
    out.push('\n');
    for _ in 0..level {
        out.push_str("  ");
    }
}

fn write_value(out: &mut String, value: &Value, style: Style, sort_keys: bool, level: usize) {
    let (item_separator, key_separator) = match style {
        Style::Compact => (",", ":"),
        Style::Spaced => (", ", ": "),
        Style::Indented => (",", ": "),
    };
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(true) => out.push_str("true"),
        Value::Bool(false) => out.push_str("false"),
        Value::Number(n) => out.push_str(if n.integer {
            n.normalized_integer()
        } else {
            &n.text
        }),
        Value::String(s) => write_string(out, s),
        Value::Array(items) => {
            if items.is_empty() {
                out.push_str("[]");
                return;
            }
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push_str(item_separator);
                }
                if style == Style::Indented {
                    newline_indent(out, level + 1);
                }
                write_value(out, item, style, sort_keys, level + 1);
            }
            if style == Style::Indented {
                newline_indent(out, level);
            }
            out.push(']');
        }
        Value::Object(map) => {
            if map.is_empty() {
                out.push_str("{}");
                return;
            }
            let mut entries: Vec<(&str, &Value)> = map.iter().collect();
            if sort_keys {
                entries.sort_by(|a, b| a.0.cmp(b.0));
            }
            out.push('{');
            for (index, (key, item)) in entries.into_iter().enumerate() {
                if index > 0 {
                    out.push_str(item_separator);
                }
                if style == Style::Indented {
                    newline_indent(out, level + 1);
                }
                write_string(out, key);
                out.push_str(key_separator);
                write_value(out, item, style, sort_keys, level + 1);
            }
            if style == Style::Indented {
                newline_indent(out, level);
            }
            out.push('}');
        }
    }
}

/// Writes a quoted string with Python's `ensure_ascii` escaping.
fn write_string(out: &mut String, text: &str) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let push_unit = |out: &mut String, unit: u16| {
        out.push_str("\\u");
        for shift in [12, 8, 4, 0] {
            out.push(HEX[usize::from((unit >> shift) & 0xF)] as char);
        }
    };
    out.push('"');
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            ' '..='~' => out.push(ch),
            _ => {
                let mut units = [0u16; 2];
                for unit in ch.encode_utf16(&mut units) {
                    push_unit(out, *unit);
                }
            }
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(text: &str) -> Value {
        parse(text).unwrap_or_else(|e| panic!("{text}: {e}"))
    }

    #[test]
    fn parses_all_value_kinds_and_whitespace() {
        let v = p(" {\"a\": [1, -2.5e3, true, false, null, \"x\\u00e9\\ud83d\\ude00\\n\"]}\n");
        let items = v
            .get("a")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        assert_eq!(items.len(), 6);
        assert_eq!(items[0].as_i64(), Some(1));
        assert!(!items[1].is_integer());
        assert_eq!(items[5].as_str(), Some("x\u{e9}\u{1f600}\n"));
    }

    #[test]
    fn rejects_duplicates_bad_syntax_non_finite_and_deep_nesting() {
        assert_eq!(
            parse(r#"{"owner":"a","owner":"b"}"#),
            Err(ParseError::DuplicateKey("owner".into()))
        );
        for bad in [
            "",
            "{",
            "[1,]",
            "{\"a\":1,}",
            "01",
            "1.",
            ".5",
            "1e",
            "NaN",
            "Infinity",
            "tru",
            "\"\\x\"",
            "\"a\nb\"",
            "\"\\ud800\"",
            "\"\\udc00\"",
            "1 2",
            "{'a':1}",
            "1e400",
            "\u{feff}{}",
        ] {
            assert!(parse(bad).is_err(), "accepted {bad:?}");
        }
        let deep = "[".repeat(MAX_DEPTH + 1) + &"]".repeat(MAX_DEPTH + 1);
        assert_eq!(parse(&deep), Err(ParseError::TooDeep));
        let ok = "[".repeat(MAX_DEPTH) + &"]".repeat(MAX_DEPTH);
        assert!(parse(&ok).is_ok());
        assert_eq!(parse_bytes(&[0xff]), Err(ParseError::Utf8));
    }

    #[test]
    fn dictionary_equality_ignores_key_order_but_strict_equality_sees_types() {
        let a = p(r#"{"file":"x","handler":"h"}"#);
        let b = p(r#"{"handler":"h","file":"x"}"#);
        assert_eq!(a, b);
        assert!(a.strict_eq(&b));
        assert_eq!(p("1"), p("1.0"));
        assert!(!p("1").strict_eq(&p("1.0")));
        assert!(p("-0").strict_eq(&p("0")));
        assert!(!p("0.0").strict_eq(&p("-0.0")));
        assert_ne!(p("0"), p("false"));
        assert_ne!(p("[1,2]"), p("[2,1]"));
        assert_ne!(p(r#"{"a":1}"#), p(r#"{"a":1,"b":null}"#));
    }

    #[test]
    fn serialisation_matches_python_json_dumps_layouts() {
        let v = p(r#"{"b":[1,{"z":null,"a":"\u00e9\"\\/"}],"a":{},"c":[],"d":-0,"e":1.50}"#);
        assert_eq!(
            to_string(&v, Style::Compact, false),
            r#"{"b":[1,{"z":null,"a":"\u00e9\"\\/"}],"a":{},"c":[],"d":0,"e":1.50}"#
        );
        assert_eq!(
            to_string(&v, Style::Spaced, true),
            r#"{"a": {}, "b": [1, {"a": "\u00e9\"\\/", "z": null}], "c": [], "d": 0, "e": 1.50}"#
        );
        assert_eq!(
            to_string(&p(r#"{"k":[1,{"x":true}],"e":[]}"#), Style::Indented, false),
            "{\n  \"k\": [\n    1,\n    {\n      \"x\": true\n    }\n  ],\n  \"e\": []\n}"
        );
        assert_eq!(
            compact(&Value::from("\u{1f600}\u{7f}\u{1}")),
            r#""\ud83d\ude00\u007f\u0001""#
        );
    }

    #[test]
    fn round_trip_preserves_foreign_values() {
        let text = r#"[{"file":"/usr/lib/qt5/qml/KeyFilters/systemUi.js","handler":"handleSystemKeys","w":1E5}]"#;
        assert_eq!(compact(&p(text)), text);
    }

    #[test]
    fn integer_accessors_respect_kind_and_range() {
        assert_eq!(p("18446744073709551615").as_u64(), Some(u64::MAX));
        assert_eq!(p("-1").as_u64(), None);
        assert_eq!(p("1.0").as_i64(), None);
        assert!(p("7").is_integer());
        assert!(!p("true").is_integer());
    }
}
