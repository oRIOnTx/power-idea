//! `file://` URI ⇄ filesystem path conversion with percent-encoding.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use lsp_types::Uri;

/// Bytes allowed unescaped in a URI path segment (RFC 3986 unreserved + `/`).
fn is_unreserved(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~' | b'/')
}

pub fn path_to_uri(path: &Path) -> Result<Uri> {
    if !path.is_absolute() {
        bail!("{} is not absolute", path.display());
    }
    let mut uri = String::from("file://");
    for &byte in path.to_string_lossy().as_bytes() {
        if is_unreserved(byte) {
            uri.push(byte as char);
        } else {
            uri.push_str(&format!("%{byte:02X}"));
        }
    }
    uri.parse()
        .map_err(|_| anyhow::anyhow!("invalid URI for {}", path.display()))
}

pub fn uri_to_path(uri: &Uri) -> Result<PathBuf> {
    let encoded = uri
        .as_str()
        .strip_prefix("file://")
        .context("not a file:// URI")?;
    let bytes = encoded.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3])?;
            decoded.push(u8::from_str_radix(hex, 16)?);
            i += 3;
        } else {
            decoded.push(bytes[i]);
            i += 1;
        }
    }
    Ok(PathBuf::from(String::from_utf8(decoded)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrips_paths_with_special_characters() {
        let path = Path::new("/Users/me/My Projects/app #1/файл.rb");
        let uri = path_to_uri(path).unwrap();
        assert!(
            uri.as_str()
                .starts_with("file:///Users/me/My%20Projects/app%20%231/")
        );
        assert_eq!(uri_to_path(&uri).unwrap(), path);
    }

    #[test]
    fn rejects_relative_and_non_file_uris() {
        assert!(path_to_uri(Path::new("relative.rb")).is_err());
        assert!(uri_to_path(&"https://example.com/a".parse().unwrap()).is_err());
    }
}
