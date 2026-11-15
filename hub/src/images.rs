//! Checking uploaded pictures, and taking the hidden data out of them.
//!
//! The app re-draws every picture before sending it, which already drops the camera's EXIF block (where the phone
//! writes the GPS position). The hub strips it again from JPEGs, so a picture sent some other way can't carry where a
//! child was.

/// The picture's type from its first bytes, or `None` when it isn't one the app shows.
pub fn sniff(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else {
        None
    }
}

/// A JPEG without its APP1 (EXIF, XMP) and comment segments. Anything it can't follow is returned as it was given.
pub fn strip_jpeg_metadata(bytes: &[u8]) -> Vec<u8> {
    if !bytes.starts_with(&[0xFF, 0xD8]) {
        return bytes.to_vec();
    }
    let mut out = Vec::with_capacity(bytes.len());
    out.extend_from_slice(&bytes[..2]);
    let mut i = 2;
    while i + 4 <= bytes.len() {
        if bytes[i] != 0xFF {
            return bytes.to_vec();
        }
        let marker = bytes[i + 1];
        // Start of scan: the picture data follows, with no more segments to look at.
        if marker == 0xDA {
            out.extend_from_slice(&bytes[i..]);
            return out;
        }
        let len = u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]) as usize;
        if len < 2 || i + 2 + len > bytes.len() {
            return bytes.to_vec();
        }
        let segment = &bytes[i..i + 2 + len];
        // APP1 (EXIF/XMP), APP13 (Photoshop/IPTC) and COM (comments) can hold places, names and dates.
        if !matches!(marker, 0xE1 | 0xED | 0xFE) {
            out.extend_from_slice(segment);
        }
        i += 2 + len;
    }
    bytes.to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn segment(marker: u8, payload: &[u8]) -> Vec<u8> {
        let mut s = vec![0xFF, marker];
        s.extend_from_slice(&((payload.len() + 2) as u16).to_be_bytes());
        s.extend_from_slice(payload);
        s
    }

    #[test]
    fn strips_exif_but_keeps_the_rest() {
        let mut jpeg = vec![0xFF, 0xD8];
        jpeg.extend(segment(0xE0, b"JFIF\0"));
        jpeg.extend(segment(0xE1, b"Exif\0\0GPS-SECRET"));
        jpeg.extend(segment(0xDB, &[1, 2, 3]));
        jpeg.extend([0xFF, 0xDA, 0, 2, 9, 9, 9, 0xFF, 0xD9]);
        let out = strip_jpeg_metadata(&jpeg);
        assert!(!out.windows(10).any(|w| w == b"GPS-SECRET"));
        assert!(out.windows(4).any(|w| w == b"JFIF"));
        assert!(out.ends_with(&[9, 9, 9, 0xFF, 0xD9]));
        assert_eq!(sniff(&out), Some("image/jpeg"));
    }

    #[test]
    fn leaves_broken_files_alone() {
        let broken = vec![0xFF, 0xD8, 0xFF, 0xE1, 0xFF, 0xFF];
        assert_eq!(strip_jpeg_metadata(&broken), broken);
    }

    #[test]
    fn sniffs_types() {
        assert_eq!(sniff(b"\x89PNG\r\n\x1a\nrest"), Some("image/png"));
        assert_eq!(sniff(b"GIF89a..."), Some("image/gif"));
        assert_eq!(sniff(b"RIFF\0\0\0\0WEBPVP8 "), Some("image/webp"));
        assert_eq!(sniff(b"<html>"), None);
    }
}
