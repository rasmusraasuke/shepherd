use std::io::{self, BufRead, Write};

use serde_json::Value;

pub fn write(writer: &mut impl Write, message: &Value) -> io::Result<()> {
    let body = serde_json::to_string(message)?;
    write!(writer, "Content-Length: {}\r\n\r\n{body}", body.len())?;
    writer.flush()
}

pub fn read(reader: &mut impl BufRead) -> io::Result<Option<Value>> {
    let mut length = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':')
            && name.eq_ignore_ascii_case("content-length")
        {
            length = value.trim().parse().ok();
        }
    }
    let length = length.ok_or_else(|| io::Error::other("No Content-Length"))?;
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    Ok(Some(serde_json::from_slice(&body)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn frames_messages_with_their_length() {
        let mut out = Vec::new();
        write(&mut out, &json!({"a": "ä"})).unwrap();
        write(&mut out, &json!([1])).unwrap();
        assert!(out.starts_with(b"Content-Length: 10\r\n\r\n{\"a\":\"\xc3\xa4\"}"));
        let mut reader = io::BufReader::new(&out[..]);
        assert_eq!(read(&mut reader).unwrap(), Some(json!({"a": "ä"})));
        assert_eq!(read(&mut reader).unwrap(), Some(json!([1])));
        assert_eq!(read(&mut reader).unwrap(), None);
    }

    #[test]
    fn skips_other_headers() {
        let text = "Content-Type: application/vscode-jsonrpc\r\ncontent-length: 2\r\n\r\n{}";
        let mut reader = io::BufReader::new(text.as_bytes());
        assert_eq!(read(&mut reader).unwrap(), Some(json!({})));
    }
}
