//! Photos and videos: what type a file is, and taking out the hidden data that could say where a child was.
//!
//! A phone writes the GPS position into a photo's EXIF block and into a video's metadata. The app takes it out before
//! anything leaves the device, and the hub does it again, so a file sent some other way can't carry it either.
//!
//! - JPEG: every EXIF, XMP, IPTC and comment segment goes. Only the rotation is kept (written back as a tiny EXIF
//!   block of its own), so the photo still shows the right way up. A Pixel "Ultra HDR" photo's gain map (a second
//!   picture after the first) goes too.
//! - PNG: the text, time and EXIF chunks go.
//! - MP4 / MOV: the location boxes (`©xyz`, `loci`, and QuickTime `mdta` location keys) are renamed `free`, which
//!   every player skips. Nothing moves, so the rest of the file stays exactly as it was.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;

/// The file's type from its first bytes, or `None` when it isn't a photo or video the app knows.
pub fn sniff(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.len() >= 12 && &bytes[4..8] == b"ftyp" {
        match &bytes[8..12] {
            b"heic" | b"heix" | b"hevc" | b"heim" | b"heis" | b"mif1" | b"msf1" => Some("image/heic"),
            b"avif" | b"avis" => Some("image/avif"),
            b"qt  " => Some("video/quicktime"),
            _ => Some("video/mp4"),
        }
    } else if bytes.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]) {
        Some("video/webm")
    } else {
        None
    }
}

/// Pictures every device can show in the chat.
pub fn is_shown_picture(kind: &str) -> bool {
    matches!(kind, "image/jpeg" | "image/png" | "image/webp" | "image/gif")
}

/// Full-size photos the app sends as they are, once cleaned: the ones whose hidden data it knows how to take out.
pub fn is_full_photo(kind: &str) -> bool {
    matches!(kind, "image/jpeg" | "image/png")
}

pub fn is_video(kind: &str) -> bool {
    matches!(kind, "video/mp4" | "video/quicktime")
}

/// Takes the hidden data out of a photo (JPEG or PNG). Other types are returned as they are.
pub fn clean_photo(bytes: &[u8]) -> Vec<u8> {
    match sniff(bytes) {
        Some("image/jpeg") => clean_jpeg(bytes),
        Some("image/png") => clean_png(bytes),
        _ => bytes.to_vec(),
    }
}

// ---- JPEG -------------------------------------------------------------------------------------------------------

/// A JPEG with no metadata but its rotation. Anything it can't follow is returned as it was given.
pub fn clean_jpeg(bytes: &[u8]) -> Vec<u8> {
    jpeg(bytes).unwrap_or_else(|| bytes.to_vec())
}

fn jpeg(bytes: &[u8]) -> Option<Vec<u8>> {
    if !bytes.starts_with(&[0xFF, 0xD8]) {
        return None;
    }
    let mut kept: Vec<&[u8]> = Vec::new();
    let mut orientation = None;
    let mut i = 2;
    loop {
        if i + 4 > bytes.len() || bytes[i] != 0xFF {
            return None;
        }
        let marker = bytes[i + 1];
        if marker == 0xFF {
            i += 1; // fill byte
            continue;
        }
        let len = u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]) as usize;
        if len < 2 || i + 2 + len > bytes.len() {
            return None;
        }
        let segment = &bytes[i..i + 2 + len];
        let payload = &segment[4..];
        if marker == 0xDA {
            break;
        }
        let keep = match marker {
            0xE1 => {
                if payload.starts_with(b"Exif\0\0") {
                    orientation = orientation.or_else(|| exif_orientation(&payload[6..]));
                }
                false
            }
            // APP2: the colour profile stays; the MPF index of an Ultra HDR gain map (and FlashPix) goes.
            0xE2 => payload.starts_with(b"ICC_PROFILE\0"),
            // APP14 (Adobe) says how the colours are stored, so it stays. APP3..APP13 and APP15 are maker data, IPTC
            // (APP13), JUMBF (APP11)...
            0xE3..=0xED | 0xEF => false,
            0xFE => false, // comment
            _ => true,
        };
        if keep {
            kept.push(segment);
        }
        i += 2 + len;
    }
    // `i` is at the first scan. Follow the scans to the end of the picture, so whatever comes after (a gain map) goes.
    let end = jpeg_end(bytes, i).unwrap_or(bytes.len());

    let mut out = Vec::with_capacity(end);
    out.extend_from_slice(&[0xFF, 0xD8]);
    let exif = orientation.filter(|&o| o != 1).map(exif_with_orientation);
    let mut segments = kept.into_iter().peekable();
    // JFIF readers want APP0 first, so the new EXIF block goes after it.
    if let Some(first) = segments.peek()
        && first[1] == 0xE0
    {
        out.extend_from_slice(first);
        segments.next();
    }
    if let Some(exif) = &exif {
        out.extend_from_slice(exif);
    }
    for segment in segments {
        out.extend_from_slice(segment);
    }
    out.extend_from_slice(&bytes[i..end]);
    Some(out)
}

/// Where the picture that starts with the scan at `start` ends: just after its EOI marker.
fn jpeg_end(bytes: &[u8], mut i: usize) -> Option<usize> {
    loop {
        // A marker segment (SOS, DHT, DRI...) between scans.
        if i + 4 > bytes.len() || bytes[i] != 0xFF {
            return None;
        }
        let marker = bytes[i + 1];
        if marker == 0xD9 {
            return Some(i + 2);
        }
        let len = u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]) as usize;
        i += 2 + len;
        if marker == 0xDA {
            // The coded data: an 0xFF in it is always followed by 0x00 or a restart marker (0xD0..=0xD7).
            loop {
                let ff = i + bytes.get(i..)?.iter().position(|&b| b == 0xFF)?;
                let next = *bytes.get(ff + 1)?;
                if next == 0x00 || (0xD0..=0xD7).contains(&next) || next == 0xFF {
                    i = ff + 1;
                    continue;
                }
                i = ff;
                break;
            }
        }
    }
}

/// The rotation (EXIF tag 0x0112) from an EXIF block's TIFF data.
fn exif_orientation(tiff: &[u8]) -> Option<u16> {
    let big = match tiff.get(0..2)? {
        b"MM" => true,
        b"II" => false,
        _ => return None,
    };
    let u16_at = |at: usize| -> Option<u16> {
        let b = tiff.get(at..at + 2)?;
        Some(if big { u16::from_be_bytes([b[0], b[1]]) } else { u16::from_le_bytes([b[0], b[1]]) })
    };
    let u32_at = |at: usize| -> Option<u32> {
        let b = tiff.get(at..at + 4)?;
        let b = [b[0], b[1], b[2], b[3]];
        Some(if big { u32::from_be_bytes(b) } else { u32::from_le_bytes(b) })
    };
    let ifd = u32_at(4)? as usize;
    let count = u16_at(ifd)? as usize;
    (0..count.min(200)).find_map(|n| {
        let entry = ifd + 2 + n * 12;
        (u16_at(entry)? == 0x0112 && u16_at(entry + 2)? == 3).then(|| u16_at(entry + 8)).flatten()
    })
    .filter(|o| (1..=8).contains(o))
}

/// An APP1 segment holding nothing but the rotation.
fn exif_with_orientation(orientation: u16) -> Vec<u8> {
    let mut payload = b"Exif\0\0MM\0\x2A\0\0\0\x08".to_vec();
    payload.extend_from_slice(&1u16.to_be_bytes()); // one entry
    payload.extend_from_slice(&[0x01, 0x12, 0x00, 0x03, 0, 0, 0, 1]);
    payload.extend_from_slice(&orientation.to_be_bytes());
    payload.extend_from_slice(&[0, 0, 0, 0, 0, 0]); // padding, then no next IFD
    let mut segment = vec![0xFF, 0xE1];
    segment.extend_from_slice(&((payload.len() + 2) as u16).to_be_bytes());
    segment.extend_from_slice(&payload);
    segment
}

// ---- PNG --------------------------------------------------------------------------------------------------------

/// A PNG without its text, time and EXIF chunks. Anything it can't follow is returned as it was given.
pub fn clean_png(bytes: &[u8]) -> Vec<u8> {
    const SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";
    if !bytes.starts_with(SIGNATURE) {
        return bytes.to_vec();
    }
    let mut out = SIGNATURE.to_vec();
    let mut i = SIGNATURE.len();
    while i + 12 <= bytes.len() {
        let len = u32::from_be_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]) as usize;
        let Some(end) = i.checked_add(12 + len).filter(|&e| e <= bytes.len()) else {
            return bytes.to_vec();
        };
        let kind = &bytes[i + 4..i + 8];
        if !matches!(kind, b"eXIf" | b"tEXt" | b"zTXt" | b"iTXt" | b"tIME") {
            out.extend_from_slice(&bytes[i..end]);
        }
        i = end;
        if kind == b"IEND" {
            return out;
        }
    }
    bytes.to_vec()
}

// ---- MP4 / MOV --------------------------------------------------------------------------------------------------

/// Takes the location out of an MP4 or MOV file, changing it in place. Says how many location boxes it found.
pub fn clean_video_file(path: &Path) -> io::Result<usize> {
    let mut file = File::options().read(true).write(true).open(path)?;
    let len = file.metadata()?.len();
    let mut at = 0u64;
    let mut found = 0;
    while at + 8 <= len {
        file.seek(SeekFrom::Start(at))?;
        let mut header = [0u8; 16];
        file.read_exact(&mut header[..8])?;
        let mut size = u32::from_be_bytes([header[0], header[1], header[2], header[3]]) as u64;
        let mut header_len = 8;
        if size == 1 {
            file.read_exact(&mut header[8..16])?;
            size = u64::from_be_bytes(header[8..16].try_into().unwrap());
            header_len = 16;
        } else if size == 0 {
            size = len - at;
        }
        if size < header_len || at + size > len {
            break;
        }
        if &header[4..8] == b"moov" {
            // The movie's index: a few MB at most, even for a long video.
            if size > 256 * 1024 * 1024 {
                return Err(io::Error::other("this video's index is too big"));
            }
            let mut moov = vec![0u8; (size - header_len) as usize];
            file.read_exact(&mut moov)?;
            let n = clean_boxes(&mut moov, Container::Plain);
            if n > 0 {
                file.seek(SeekFrom::Start(at + header_len))?;
                file.write_all(&moov)?;
                file.flush()?;
            }
            found += n;
        }
        at += size;
    }
    Ok(found)
}

/// The same, on a whole file in memory.
pub fn clean_video(bytes: &mut [u8]) -> usize {
    clean_boxes(bytes, Container::Plain)
}

#[derive(Clone, Copy)]
enum Container {
    Plain,
    /// `meta` in ISO files is a "full box": four bytes of version and flags before its children.
    Meta,
}

const LOCATION_BOXES: &[&[u8; 4]] = &[b"\xA9xyz", b"loci", b"\xA9loc"];
const CONTAINERS: &[&[u8; 4]] = &[b"moov", b"trak", b"udta", b"mdia", b"minf", b"ilst"];

fn clean_boxes(data: &mut [u8], container: Container) -> usize {
    let start = match container {
        // QuickTime writes `meta` without the version: then its first child's type comes straight away.
        Container::Meta if data.len() >= 8 && data[0..4] == [0, 0, 0, 0] => 4,
        _ => 0,
    };
    let mut found = 0;
    let mut location_keys: Vec<u32> = Vec::new();
    let mut i = start;
    while i + 8 <= data.len() {
        let size = u32::from_be_bytes([data[i], data[i + 1], data[i + 2], data[i + 3]]) as usize;
        let (size, header) = match size {
            0 => (data.len() - i, 8),
            1 if i + 16 <= data.len() => {
                (u64::from_be_bytes(data[i + 8..i + 16].try_into().unwrap()) as usize, 16)
            }
            _ => (size, 8),
        };
        if size < header || i + size > data.len() {
            break;
        }
        let kind: [u8; 4] = data[i + 4..i + 8].try_into().unwrap();
        let body = i + header..i + size;
        if LOCATION_BOXES.contains(&&kind) {
            data[i + 4..i + 8].copy_from_slice(b"free");
            found += 1;
        } else if &kind == b"keys" {
            location_keys = mdta_location_keys(&data[body.clone()]);
        } else if &kind == b"ilst" && !location_keys.is_empty() {
            found += clean_ilst(&mut data[body.clone()], &location_keys);
        } else if &kind == b"meta" {
            found += clean_boxes(&mut data[body.clone()], Container::Meta);
        } else if CONTAINERS.contains(&&kind) {
            found += clean_boxes(&mut data[body.clone()], Container::Plain);
        }
        i += size;
    }
    found
}

/// The (1-based) indexes of QuickTime metadata keys naming a location, such as
/// `com.apple.quicktime.location.ISO6709`.
fn mdta_location_keys(keys: &[u8]) -> Vec<u32> {
    let mut out = Vec::new();
    let mut i = 8; // version, flags, entry count
    let mut index = 1;
    while i + 8 <= keys.len() {
        let size = u32::from_be_bytes([keys[i], keys[i + 1], keys[i + 2], keys[i + 3]]) as usize;
        if size < 8 || i + size > keys.len() {
            break;
        }
        let name = String::from_utf8_lossy(&keys[i + 8..i + size]).to_ascii_lowercase();
        if name.contains("location") || name.contains("gps") {
            out.push(index);
        }
        index += 1;
        i += size;
    }
    out
}

/// `ilst` items under a location key become `free`.
fn clean_ilst(ilst: &mut [u8], keys: &[u32]) -> usize {
    let mut found = 0;
    let mut i = 0;
    while i + 8 <= ilst.len() {
        let size = u32::from_be_bytes([ilst[i], ilst[i + 1], ilst[i + 2], ilst[i + 3]]) as usize;
        if size < 8 || i + size > ilst.len() {
            break;
        }
        let key = u32::from_be_bytes([ilst[i + 4], ilst[i + 5], ilst[i + 6], ilst[i + 7]]);
        if keys.contains(&key) {
            ilst[i + 4..i + 8].copy_from_slice(b"free");
            found += 1;
        }
        i += size;
    }
    found
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

    fn exif(orientation: u16) -> Vec<u8> {
        // Little-endian, like most phones: one IFD with the orientation and a "GPS" pointer.
        let mut p = b"Exif\0\0II\x2A\0\x08\0\0\0".to_vec();
        p.extend_from_slice(&2u16.to_le_bytes());
        p.extend_from_slice(&[0x12, 0x01, 3, 0, 1, 0, 0, 0]);
        p.extend_from_slice(&orientation.to_le_bytes());
        p.extend_from_slice(&[0, 0]);
        p.extend_from_slice(&[0x25, 0x88, 4, 0, 1, 0, 0, 0, 0, 0, 0, 0]);
        p.extend_from_slice(&[0, 0, 0, 0]);
        p.extend_from_slice(b"GPS-SECRET");
        segment(0xE1, &p)
    }

    fn jpeg(orientation: u16) -> Vec<u8> {
        let mut j = vec![0xFF, 0xD8];
        j.extend(segment(0xE0, b"JFIF\0"));
        j.extend(exif(orientation));
        j.extend(segment(0xE1, b"http://ns.adobe.com/xap/1.0/\0<x:xmpmeta>GPS-SECRET"));
        j.extend(segment(0xE2, b"ICC_PROFILE\0\x01\x01colours"));
        j.extend(segment(0xE2, b"MPF\0index"));
        j.extend(segment(0xFE, b"a comment"));
        j.extend(segment(0xDB, &[1, 2, 3]));
        // Coded data with a stuffed 0xFF00 and a restart marker, then the end, then a gain map.
        j.extend([0xFF, 0xDA, 0, 2, 9, 0xFF, 0x00, 9, 0xFF, 0xD0, 9, 0xFF, 0xD9]);
        j.extend([0xFF, 0xD8, 0xFF, 0xE1, 0, 4, b'G', b'M', 0xFF, 0xD9]);
        j
    }

    fn contains(hay: &[u8], needle: &[u8]) -> bool {
        hay.windows(needle.len()).any(|w| w == needle)
    }

    #[test]
    fn jpeg_keeps_only_the_rotation_and_the_colours() {
        let out = clean_jpeg(&jpeg(6));
        assert!(!contains(&out, b"GPS-SECRET"));
        assert!(!contains(&out, b"MPF\0"));
        assert!(!contains(&out, b"a comment"));
        assert!(contains(&out, b"ICC_PROFILE"));
        assert!(contains(&out, b"JFIF"));
        assert!(out.ends_with(&[9, 0xFF, 0xD0, 9, 0xFF, 0xD9]), "the gain map after the picture goes");
        // The rotation survives, right after APP0.
        assert_eq!(&out[2..4], &[0xFF, 0xE0]);
        let app1 = 2 + 2 + u16::from_be_bytes([out[4], out[5]]) as usize;
        assert_eq!(&out[app1..app1 + 2], &[0xFF, 0xE1]);
        assert_eq!(exif_orientation(&out[app1 + 10..]), Some(6));
        assert_eq!(sniff(&out), Some("image/jpeg"));
    }

    #[test]
    fn upright_jpegs_get_no_exif_at_all() {
        let out = clean_jpeg(&jpeg(1));
        assert!(!contains(&out, b"Exif"));
    }

    #[test]
    fn broken_jpegs_are_left_alone() {
        let broken = vec![0xFF, 0xD8, 0xFF, 0xE1, 0xFF, 0xFF];
        assert_eq!(clean_jpeg(&broken), broken);
    }

    fn chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut c = (data.len() as u32).to_be_bytes().to_vec();
        c.extend_from_slice(kind);
        c.extend_from_slice(data);
        c.extend_from_slice(&[0, 0, 0, 0]);
        c
    }

    #[test]
    fn png_loses_its_text() {
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        png.extend(chunk(b"IHDR", &[0; 13]));
        png.extend(chunk(b"tEXt", b"Location\0home"));
        png.extend(chunk(b"eXIf", b"MM GPS"));
        png.extend(chunk(b"IDAT", b"pixels"));
        png.extend(chunk(b"IEND", b""));
        let out = clean_png(&png);
        assert!(!contains(&out, b"home"));
        assert!(!contains(&out, b"GPS"));
        assert!(contains(&out, b"pixels"));
        assert!(contains(&out, b"IEND"));
    }

    fn mp4_box(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut b = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        b.extend_from_slice(kind);
        b.extend_from_slice(body);
        b
    }

    fn video() -> Vec<u8> {
        let xyz = mp4_box(b"\xA9xyz", b"\0\x11\x15\xc7+40.7128-074.0060/");
        let udta = mp4_box(b"udta", &xyz);
        let mut keys_body = vec![0, 0, 0, 0, 0, 0, 0, 2];
        keys_body.extend(mp4_box(b"mdta", b"com.android.version"));
        keys_body.extend(mp4_box(b"mdta", b"com.apple.quicktime.location.ISO6709"));
        let keys = mp4_box(b"keys", &keys_body);
        let mut items = mp4_box(b"\0\0\0\x01", b"14");
        items.extend(mp4_box(b"\0\0\0\x02", b"+51.5-000.1/"));
        let ilst = mp4_box(b"ilst", &items);
        let mut meta_body = vec![0, 0, 0, 0];
        meta_body.extend(mp4_box(b"hdlr", b"mdta"));
        meta_body.extend(keys);
        meta_body.extend(ilst);
        let meta = mp4_box(b"meta", &meta_body);
        let mut moov_body = mp4_box(b"mvhd", &[0; 20]);
        moov_body.extend(udta);
        moov_body.extend(meta);
        let mut file = mp4_box(b"ftyp", b"isom\0\0\0\0");
        file.extend(mp4_box(b"mdat", b"frames"));
        file.extend(mp4_box(b"moov", &moov_body));
        file
    }

    #[test]
    fn video_locations_become_free_boxes() {
        let mut v = video();
        let before = v.len();
        assert_eq!(sniff(&v), Some("video/mp4"));
        let mut in_memory = v.clone();
        assert_eq!(clean_video(&mut in_memory), 2);
        let dir = std::env::temp_dir().join(format!("kit-media-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("v.mp4");
        std::fs::write(&path, &v).unwrap();
        assert_eq!(clean_video_file(&path).unwrap(), 2);
        v = std::fs::read(&path).unwrap();
        std::fs::remove_dir_all(&dir).ok();
        assert_eq!(v.len(), before);
        assert!(!contains(&v, b"\xA9xyz"));
        assert!(!contains(&v, b"\0\0\0\x02+51"), "the location key's value is renamed");
        assert!(contains(&v, b"\0\0\0\x01"), "other keys stay");
        assert!(contains(&v, b"frames"));
        assert_eq!(v, in_memory);
    }

    #[test]
    fn sniffs_types() {
        assert_eq!(sniff(b"\x89PNG\r\n\x1a\nrest"), Some("image/png"));
        assert_eq!(sniff(b"GIF89a..."), Some("image/gif"));
        assert_eq!(sniff(b"RIFF\0\0\0\0WEBPVP8 "), Some("image/webp"));
        assert_eq!(sniff(b"\0\0\0\x18ftypheic...."), Some("image/heic"));
        assert_eq!(sniff(b"\0\0\0\x18ftypqt  ...."), Some("video/quicktime"));
        assert_eq!(sniff(b"<html>"), None);
    }
}
