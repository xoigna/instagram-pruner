use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use std::io::{stdout, Write};

pub fn copy(text: &str) -> std::io::Result<()> {
    let encoded = BASE64.encode(text.as_bytes());
    let mut out = stdout();
    out.write_all(b"\x1b]52;c;")?;
    out.write_all(encoded.as_bytes())?;
    out.write_all(b"\x07")?;
    out.flush()
}
