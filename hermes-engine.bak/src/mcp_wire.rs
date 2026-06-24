use serde_json::Value;
use std::io::{self, BufRead, Write};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WireMode {
    Auto,
    Line,
    Framed,
}

pub fn read_next_message(
    reader: &mut impl BufRead,
    mode: &mut WireMode,
) -> io::Result<Option<String>> {
    match mode {
        WireMode::Auto => read_auto_message(reader, mode),
        WireMode::Line => read_line_message(reader),
        WireMode::Framed => read_framed_message(reader),
    }
}

pub fn write_json(writer: &mut impl Write, mode: WireMode, payload: &Value) -> io::Result<()> {
    let body = serde_json::to_string(payload)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    match mode {
        WireMode::Framed => {
            write!(writer, "Content-Length: {}\r\n\r\n{}", body.len(), body)?;
        }
        WireMode::Auto | WireMode::Line => {
            writeln!(writer, "{body}")?;
        }
    }
    writer.flush()?;
    Ok(())
}

fn read_auto_message(reader: &mut impl BufRead, mode: &mut WireMode) -> io::Result<Option<String>> {
    let mut first_line = String::new();
    loop {
        first_line.clear();
        let bytes = reader.read_line(&mut first_line)?;
        if bytes == 0 {
            return Ok(None);
        }
        if first_line.trim().is_empty() {
            continue;
        }
        if is_content_length_header(&first_line) {
            *mode = WireMode::Framed;
            return read_framed_message_with_header(reader, &first_line).map(Some);
        }
        *mode = WireMode::Line;
        return Ok(Some(first_line.trim().to_string()));
    }
}

fn read_line_message(reader: &mut impl BufRead) -> io::Result<Option<String>> {
    let mut line = String::new();
    loop {
        line.clear();
        let bytes = reader.read_line(&mut line)?;
        if bytes == 0 {
            return Ok(None);
        }
        if line.trim().is_empty() {
            continue;
        }
        return Ok(Some(line.trim().to_string()));
    }
}

fn read_framed_message(reader: &mut impl BufRead) -> io::Result<Option<String>> {
    let mut header_line = String::new();
    loop {
        header_line.clear();
        let bytes = reader.read_line(&mut header_line)?;
        if bytes == 0 {
            return Ok(None);
        }
        if header_line.trim().is_empty() {
            continue;
        }
        return read_framed_message_with_header(reader, &header_line).map(Some);
    }
}

fn read_framed_message_with_header(
    reader: &mut impl BufRead,
    first_header: &str,
) -> io::Result<String> {
    let len = parse_content_length(first_header)?;
    read_remaining_headers(reader)?;

    let mut body = vec![0u8; len];
    reader.read_exact(&mut body)?;
    String::from_utf8(body).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
}

fn read_remaining_headers(reader: &mut impl BufRead) -> io::Result<()> {
    let mut line = String::new();
    loop {
        line.clear();
        let bytes = reader.read_line(&mut line)?;
        if bytes == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "unexpected EOF while reading MCP headers",
            ));
        }
        if line.trim().is_empty() {
            return Ok(());
        }
    }
}

fn parse_content_length(line: &str) -> io::Result<usize> {
    let (name, value) = line
        .split_once(':')
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing Content-Length"))?;
    if !name.trim().eq_ignore_ascii_case("content-length") {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid MCP header; expected Content-Length",
        ));
    }
    value
        .trim()
        .parse::<usize>()
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
}

fn is_content_length_header(line: &str) -> bool {
    line.split_once(':')
        .map(|(name, _)| name.trim().eq_ignore_ascii_case("content-length"))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn auto_mode_reads_line_json() {
        let input = b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\"}\n";
        let mut reader = Cursor::new(input.as_slice());
        let mut mode = WireMode::Auto;
        let msg = read_next_message(&mut reader, &mut mode).unwrap().unwrap();
        assert_eq!(mode, WireMode::Line);
        assert!(msg.contains("\"initialize\""));
    }

    #[test]
    fn auto_mode_reads_framed_json() {
        let body = r#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#;
        let input = format!("Content-Length: {}\r\n\r\n{}", body.len(), body);
        let mut reader = Cursor::new(input.into_bytes());
        let mut mode = WireMode::Auto;
        let msg = read_next_message(&mut reader, &mut mode).unwrap().unwrap();
        assert_eq!(mode, WireMode::Framed);
        assert_eq!(msg, body);
    }

    #[test]
    fn framed_write_includes_content_length() {
        let mut out = Vec::new();
        let payload = serde_json::json!({"jsonrpc":"2.0","id":1,"result":{"ok":true}});
        write_json(&mut out, WireMode::Framed, &payload).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.starts_with("Content-Length: "));
        assert!(text.contains("\r\n\r\n"));
        assert!(text.contains("\"ok\":true"));
    }
}
