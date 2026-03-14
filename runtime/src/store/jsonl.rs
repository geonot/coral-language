use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, BufWriter, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use super::binary::StoredValue;
use super::uuid7::Uuid7;

pub struct JsonlWriter {
    path: PathBuf,
    file: BufWriter<File>,
    write_offset: u64,
    line_count: u64,
}

impl JsonlWriter {
    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref().to_path_buf();

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let (file, write_offset, line_count) = if path.exists() {
            let mut file = OpenOptions::new().read(true).write(true).open(&path)?;

            let mut line_count = 0u64;
            let reader = BufReader::new(&file);
            for _ in reader.lines() {
                line_count += 1;
            }

            let offset = file.seek(SeekFrom::End(0))?;

            (file, offset, line_count)
        } else {
            let file = OpenOptions::new().write(true).create(true).open(&path)?;
            (file, 0, 0)
        };

        Ok(Self {
            path,
            file: BufWriter::new(file),
            write_offset,
            line_count,
        })
    }

    pub fn write_record(
        &mut self,
        index: u64,
        version: u32,
        uuid: &Uuid7,
        created_at: i64,
        updated_at: i64,
        deleted_at: i64,
        fields: &[(String, StoredValue)],
    ) -> io::Result<(u64, u32)> {
        let offset = self.write_offset;

        let mut json = String::from("{");

        json.push_str(&format!("\"_index\":{}", index));
        json.push_str(&format!(",\"_uuid\":\"{}\"", uuid));
        json.push_str(&format!(",\"_version\":{}", version));
        json.push_str(&format!(",\"_created_at\":{}", created_at));
        json.push_str(&format!(",\"_updated_at\":{}", updated_at));
        if deleted_at >= 0 {
            json.push_str(&format!(",\"_deleted_at\":{}", deleted_at));
        }

        for (name, value) in fields {
            json.push(',');
            json.push('"');
            escape_json_string(&mut json, name);
            json.push_str("\":");
            value_to_json(&mut json, value);
        }

        json.push('}');
        json.push('\n');

        let bytes = json.as_bytes();
        self.file.write_all(bytes)?;
        self.file.flush()?;

        let length = bytes.len() as u32;
        self.write_offset += length as u64;
        self.line_count += 1;

        Ok((offset, length))
    }

    pub fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }

    pub fn write_offset(&self) -> u64 {
        self.write_offset
    }

    pub fn line_count(&self) -> u64 {
        self.line_count
    }
}

pub struct JsonlReader {
    path: PathBuf,
}

impl JsonlReader {
    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        if !path.exists() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "jsonl file not found",
            ));
        }
        Ok(Self { path })
    }

    pub fn read_record(&self, offset: u64) -> io::Result<JsonlRecord> {
        let mut file = File::open(&self.path)?;
        file.seek(SeekFrom::Start(offset))?;

        let mut reader = BufReader::new(file);
        let mut line = String::new();
        reader.read_line(&mut line)?;

        parse_jsonl_record(&line)
    }

    pub fn iter(&self) -> io::Result<JsonlIterator> {
        let file = File::open(&self.path)?;
        Ok(JsonlIterator {
            reader: BufReader::new(file),
            offset: 0,
        })
    }
}

pub struct JsonlIterator {
    reader: BufReader<File>,
    offset: u64,
}

impl Iterator for JsonlIterator {
    type Item = io::Result<(u64, JsonlRecord)>;

    fn next(&mut self) -> Option<Self::Item> {
        let mut line = String::new();
        match self.reader.read_line(&mut line) {
            Ok(0) => None,
            Ok(n) => {
                let offset = self.offset;
                self.offset += n as u64;

                if line.trim().is_empty() {
                    return self.next();
                }

                Some(parse_jsonl_record(&line).map(|r| (offset, r)))
            }
            Err(e) => Some(Err(e)),
        }
    }
}

#[derive(Debug, Clone)]
pub struct JsonlRecord {
    pub index: u64,
    pub version: u32,
    pub uuid: Uuid7,
    pub created_at: i64,
    pub updated_at: i64,
    pub deleted_at: i64,
    pub fields: HashMap<String, StoredValue>,
}

fn parse_jsonl_record(line: &str) -> io::Result<JsonlRecord> {
    let line = line.trim();
    if !line.starts_with('{') || !line.ends_with('}') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid JSON object",
        ));
    }

    let mut parser = JsonParser::new(&line[1..line.len() - 1]);
    let mut fields = HashMap::new();
    let mut index = 0u64;
    let mut version = 1u32;
    let mut uuid = Uuid7::new();
    let mut created_at = 0i64;
    let mut updated_at = 0i64;
    let mut deleted_at = -1i64;

    loop {
        parser.skip_whitespace();
        if parser.is_empty() {
            break;
        }

        let key = parser.parse_string()?;
        parser.skip_whitespace();
        parser.expect_char(':')?;
        parser.skip_whitespace();

        let value = parser.parse_value()?;

        match key.as_str() {
            "_index" => {
                if let StoredValue::Int(i) = value {
                    index = i as u64;
                }
            }
            "_version" => {
                if let StoredValue::Int(i) = value {
                    version = i as u32;
                }
            }
            "_uuid" => {
                if let StoredValue::String(s) = value {
                    uuid = Uuid7::parse(&s)?;
                }
            }
            "_created_at" => {
                if let StoredValue::Int(i) = value {
                    created_at = i;
                }
            }
            "_updated_at" => {
                if let StoredValue::Int(i) = value {
                    updated_at = i;
                }
            }
            "_deleted_at" => {
                if let StoredValue::Int(i) = value {
                    deleted_at = i;
                }
            }
            _ => {
                fields.insert(key, value);
            }
        }

        parser.skip_whitespace();
        if !parser.is_empty() {
            parser.expect_char(',')?;
        }
    }

    Ok(JsonlRecord {
        index,
        version,
        uuid,
        created_at,
        updated_at,
        deleted_at,
        fields,
    })
}

struct JsonParser<'a> {
    input: &'a str,
    pos: usize,
}

impl<'a> JsonParser<'a> {
    fn new(input: &'a str) -> Self {
        Self { input, pos: 0 }
    }

    fn is_empty(&self) -> bool {
        self.pos >= self.input.len()
    }

    fn peek(&self) -> Option<char> {
        self.input[self.pos..].chars().next()
    }

    fn advance(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += c.len_utf8();
        Some(c)
    }

    fn skip_whitespace(&mut self) {
        while let Some(c) = self.peek() {
            if c.is_whitespace() {
                self.advance();
            } else {
                break;
            }
        }
    }

    fn expect_char(&mut self, expected: char) -> io::Result<()> {
        match self.advance() {
            Some(c) if c == expected => Ok(()),
            Some(c) => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("expected '{}', got '{}'", expected, c),
            )),
            None => Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                format!("expected '{}'", expected),
            )),
        }
    }

    fn parse_string(&mut self) -> io::Result<String> {
        self.expect_char('"')?;

        let mut result = String::new();
        loop {
            match self.advance() {
                Some('"') => break,
                Some('\\') => match self.advance() {
                    Some('n') => result.push('\n'),
                    Some('r') => result.push('\r'),
                    Some('t') => result.push('\t'),
                    Some('\\') => result.push('\\'),
                    Some('"') => result.push('"'),
                    Some('/') => result.push('/'),
                    Some('u') => {
                        let mut hex = String::new();
                        for _ in 0..4 {
                            match self.advance() {
                                Some(c) => hex.push(c),
                                None => {
                                    return Err(io::Error::new(
                                        io::ErrorKind::UnexpectedEof,
                                        "truncated unicode escape",
                                    ));
                                }
                            }
                        }
                        if let Ok(code) = u32::from_str_radix(&hex, 16) {
                            if let Some(c) = char::from_u32(code) {
                                result.push(c);
                            }
                        }
                    }
                    Some(c) => result.push(c),
                    None => {
                        return Err(io::Error::new(
                            io::ErrorKind::UnexpectedEof,
                            "truncated escape",
                        ));
                    }
                },
                Some(c) => result.push(c),
                None => {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "unterminated string",
                    ));
                }
            }
        }

        Ok(result)
    }

    fn parse_number(&mut self) -> io::Result<StoredValue> {
        let start = self.pos;
        let mut has_dot = false;
        let mut has_exp = false;

        if self.peek() == Some('-') {
            self.advance();
        }

        while let Some(c) = self.peek() {
            if c.is_ascii_digit() {
                self.advance();
            } else {
                break;
            }
        }

        if self.peek() == Some('.') {
            has_dot = true;
            self.advance();
            while let Some(c) = self.peek() {
                if c.is_ascii_digit() {
                    self.advance();
                } else {
                    break;
                }
            }
        }

        if let Some('e' | 'E') = self.peek() {
            has_exp = true;
            self.advance();
            if let Some('+' | '-') = self.peek() {
                self.advance();
            }
            while let Some(c) = self.peek() {
                if c.is_ascii_digit() {
                    self.advance();
                } else {
                    break;
                }
            }
        }

        let num_str = &self.input[start..self.pos];

        if has_dot || has_exp {
            num_str
                .parse::<f64>()
                .map(StoredValue::Float)
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid float"))
        } else {
            num_str
                .parse::<i64>()
                .map(StoredValue::Int)
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid integer"))
        }
    }

    fn parse_value(&mut self) -> io::Result<StoredValue> {
        match self.peek() {
            Some('"') => self.parse_string().map(StoredValue::String),
            Some(c) if c == '-' || c.is_ascii_digit() => self.parse_number(),
            Some('t') => {
                self.parse_keyword("true")?;
                Ok(StoredValue::Bool(true))
            }
            Some('f') => {
                self.parse_keyword("false")?;
                Ok(StoredValue::Bool(false))
            }
            Some('n') => {
                self.parse_keyword("null")?;
                Ok(StoredValue::None)
            }
            Some('[') => self.parse_array(),
            Some('{') => self.parse_object(),
            Some(c) => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unexpected character: '{}'", c),
            )),
            None => Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "unexpected end of JSON",
            )),
        }
    }

    fn parse_keyword(&mut self, keyword: &str) -> io::Result<()> {
        for expected in keyword.chars() {
            match self.advance() {
                Some(c) if c == expected => {}
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("expected '{}'", keyword),
                    ));
                }
            }
        }
        Ok(())
    }

    fn parse_array(&mut self) -> io::Result<StoredValue> {
        self.expect_char('[')?;
        let mut items = Vec::new();

        loop {
            self.skip_whitespace();
            if self.peek() == Some(']') {
                self.advance();
                break;
            }

            items.push(self.parse_value()?);

            self.skip_whitespace();
            match self.peek() {
                Some(',') => {
                    self.advance();
                }
                Some(']') => {}
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "expected ',' or ']'",
                    ));
                }
            }
        }

        Ok(StoredValue::List(items))
    }

    fn parse_object(&mut self) -> io::Result<StoredValue> {
        self.expect_char('{')?;
        let mut pairs = Vec::new();

        loop {
            self.skip_whitespace();
            if self.peek() == Some('}') {
                self.advance();
                break;
            }

            let key = self.parse_string()?;
            self.skip_whitespace();
            self.expect_char(':')?;
            self.skip_whitespace();
            let value = self.parse_value()?;

            pairs.push((key, value));

            self.skip_whitespace();
            match self.peek() {
                Some(',') => {
                    self.advance();
                }
                Some('}') => {}
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "expected ',' or '}'",
                    ));
                }
            }
        }

        Ok(StoredValue::Map(pairs))
    }
}

/// Escape a string for JSON
fn escape_json_string(output: &mut String, s: &str) {
    for c in s.chars() {
        match c {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            c if c.is_control() => {
                output.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => output.push(c),
        }
    }
}

fn value_to_json(output: &mut String, value: &StoredValue) {
    match value {
        StoredValue::Unit => output.push_str("null"),
        StoredValue::None => output.push_str("null"),
        StoredValue::Bool(b) => output.push_str(if *b { "true" } else { "false" }),
        StoredValue::Int(i) => output.push_str(&i.to_string()),
        StoredValue::Float(f) => {
            if f.is_nan() {
                output.push_str("null");
            } else if f.is_infinite() {
                output.push_str("null");
            } else {
                output.push_str(&f.to_string());
            }
        }
        StoredValue::String(s) => {
            output.push('"');
            escape_json_string(output, s);
            output.push('"');
        }
        StoredValue::Bytes(b) => {
            output.push('"');
            output.push_str(&base64_encode(b));
            output.push('"');
        }
        StoredValue::List(items) => {
            output.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    output.push(',');
                }
                value_to_json(output, item);
            }
            output.push(']');
        }
        StoredValue::Map(pairs) => {
            output.push('{');
            for (i, (k, v)) in pairs.iter().enumerate() {
                if i > 0 {
                    output.push(',');
                }
                output.push('"');
                escape_json_string(output, k);
                output.push_str("\":");
                value_to_json(output, v);
            }
            output.push('}');
        }
    }
}

fn base64_encode(data: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    let mut result = String::with_capacity((data.len() * 4 + 2) / 3);

    for chunk in data.chunks(3) {
        let b0 = chunk[0] as usize;
        let b1 = chunk.get(1).copied().unwrap_or(0) as usize;
        let b2 = chunk.get(2).copied().unwrap_or(0) as usize;

        let n = (b0 << 16) | (b1 << 8) | b2;

        result.push(ALPHABET[(n >> 18) & 0x3F] as char);
        result.push(ALPHABET[(n >> 12) & 0x3F] as char);

        if chunk.len() > 1 {
            result.push(ALPHABET[(n >> 6) & 0x3F] as char);
        } else {
            result.push('=');
        }

        if chunk.len() > 2 {
            result.push(ALPHABET[n & 0x3F] as char);
        } else {
            result.push('=');
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_jsonl_roundtrip() {
        let path = "/tmp/coral_test.jsonl";
        let _ = fs::remove_file(path);

        let uuid = Uuid7::new();
        let fields = vec![
            ("name".to_string(), StoredValue::String("Alice".to_string())),
            ("age".to_string(), StoredValue::Int(30)),
            ("active".to_string(), StoredValue::Bool(true)),
        ];

        let (offset, _length) = {
            let mut writer = JsonlWriter::open(path).unwrap();
            writer
                .write_record(1, 1, &uuid, 1000, 1000, -1, &fields)
                .unwrap()
        };

        {
            let reader = JsonlReader::open(path).unwrap();
            let record = reader.read_record(offset).unwrap();

            assert_eq!(record.index, 1);
            assert_eq!(record.version, 1);
            assert_eq!(record.uuid, uuid);
            assert_eq!(record.created_at, 1000);
            assert!(record.fields.contains_key("name"));
            assert_eq!(
                record.fields.get("name"),
                Some(&StoredValue::String("Alice".to_string()))
            );
        }

        let _ = fs::remove_file(path);
    }

    #[test]
    fn test_json_parser() {
        let input = r#"{"_index":42,"name":"Test","numbers":[1,2,3],"nested":{"a":1}}"#;
        let record = parse_jsonl_record(input).unwrap();

        assert_eq!(record.index, 42);
        assert_eq!(
            record.fields.get("name"),
            Some(&StoredValue::String("Test".to_string()))
        );
        assert!(matches!(
            record.fields.get("numbers"),
            Some(StoredValue::List(_))
        ));
        assert!(matches!(
            record.fields.get("nested"),
            Some(StoredValue::Map(_))
        ));
    }

    #[test]
    fn test_escape_json() {
        let mut output = String::new();
        escape_json_string(&mut output, "hello\nworld\t\"test\"");
        assert_eq!(output, "hello\\nworld\\t\\\"test\\\"");
    }
}
