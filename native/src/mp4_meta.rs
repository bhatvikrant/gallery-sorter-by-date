//! Minimal ISO-BMFF (`mp4`/`mov`/`m4v`/`3gp`) parser focused on extracting the
//! `mvhd` creation time and, where present, the QuickTime
//! `com.apple.quicktime.creationdate` user-data key.
//!
//! We only read the boxes we need (no codec/track decode), streaming through a
//! `BufReader`, so a multi-GB video costs a few KB of I/O. This avoids the
//! per-file `ffprobe` process spawn that dominates the legacy `src/metadata.ts`
//! video path.

use crate::types::DateSource;
use chrono::{DateTime, Local, TimeZone, Utc};
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

/// Seconds between `1904-01-01T00:00:00Z` (ISO-BMFF epoch) and the unix epoch.
const EPOCH_OFFSET_SECS: i64 = 2_082_844_800;

/// Top-level scan budget. We refuse to walk pathological files that try to
/// trick us into reading the whole stream looking for a `moov` that doesn't
/// exist. ~256 MB is plenty for any well-formed file (most have `moov` near
/// the start or end).
const MAX_TOPLEVEL_WALK: u64 = 256 * 1024 * 1024;

pub fn extract_mp4_date(path: &Path) -> Option<(DateTime<Local>, DateSource)> {
    let file = File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let mut r = BufReader::with_capacity(64 * 1024, file);

    let (mvhd_ts, qt_local) = walk_toplevel(&mut r, len).ok().flatten()?;

    if let Some(s) = qt_local {
        if let Some(dt) = parse_qt_date(&s) {
            return Some((dt.with_timezone(&Local), DateSource::QuickTime));
        }
    }
    if let Some(secs) = mvhd_ts {
        let unix = secs as i64 - EPOCH_OFFSET_SECS;
        if let Some(dt) = Utc.timestamp_opt(unix, 0).single() {
            return Some((dt.with_timezone(&Local), DateSource::Mp4Box));
        }
    }
    None
}

/// Walks top-level boxes looking for `moov`. Returns
/// `(mvhd_creation_time_secs, com.apple.quicktime.creationdate_string)`.
fn walk_toplevel<R: Read + Seek>(
    r: &mut R,
    file_len: u64,
) -> std::io::Result<Option<(Option<u64>, Option<String>)>> {
    let mut pos = 0u64;
    while pos < file_len.min(MAX_TOPLEVEL_WALK) {
        r.seek(SeekFrom::Start(pos))?;
        let (size, kind, hdr_len) = match read_box_header(r)? {
            Some(h) => h,
            None => return Ok(None),
        };
        let payload_end = pos
            .checked_add(size)
            .unwrap_or(file_len)
            .min(file_len);

        if &kind == b"moov" {
            // Position is just after the header.
            return parse_moov(r, payload_end - pos - hdr_len).map(Some);
        }
        if size == 0 {
            return Ok(None);
        }
        pos = payload_end;
    }
    Ok(None)
}

/// Returns `(box_size_including_header, type_fourcc, header_len)`.
fn read_box_header<R: Read>(r: &mut R) -> std::io::Result<Option<(u64, [u8; 4], u64)>> {
    let mut size_buf = [0u8; 4];
    if r.read(&mut size_buf)? < 4 {
        return Ok(None);
    }
    let mut kind = [0u8; 4];
    if r.read(&mut kind)? < 4 {
        return Ok(None);
    }
    let size32 = u32::from_be_bytes(size_buf);
    let (size, header_len) = match size32 {
        1 => {
            let mut large = [0u8; 8];
            r.read_exact(&mut large)?;
            (u64::from_be_bytes(large), 16u64)
        }
        // Box extends to EOF; we model that as 0 to bail upstream.
        0 => (0u64, 8u64),
        n => (u64::from(n), 8u64),
    };
    Ok(Some((size, kind, header_len)))
}

fn parse_moov<R: Read + Seek>(
    r: &mut R,
    payload_len: u64,
) -> std::io::Result<(Option<u64>, Option<String>)> {
    let start = r.stream_position()?;
    let mut mvhd_ts: Option<u64> = None;
    let mut qt_date: Option<String> = None;

    let mut pos = 0u64;
    while pos + 8 <= payload_len {
        r.seek(SeekFrom::Start(start + pos))?;
        let (size, kind, hdr_len) = match read_box_header(r)? {
            Some(h) => h,
            None => break,
        };
        if size < hdr_len {
            break;
        }
        let inner_payload = size - hdr_len;
        match &kind {
            b"mvhd" => {
                mvhd_ts = read_mvhd_creation(r, inner_payload).ok();
            }
            b"udta" => {
                qt_date = parse_udta(r, inner_payload).ok().flatten();
            }
            _ => {}
        }
        pos += size;
    }
    Ok((mvhd_ts, qt_date))
}

/// `mvhd` layout (after the 8-byte box header):
/// 1 byte version, 3 bytes flags, then v0: u32 creation_time / u32 mod_time
/// or v1: u64 creation_time / u64 mod_time. Times are in seconds since
/// `1904-01-01T00:00:00Z`.
fn read_mvhd_creation<R: Read>(r: &mut R, _payload_len: u64) -> std::io::Result<u64> {
    let mut hdr = [0u8; 4];
    r.read_exact(&mut hdr)?;
    let version = hdr[0];
    if version == 1 {
        let mut buf = [0u8; 8];
        r.read_exact(&mut buf)?;
        Ok(u64::from_be_bytes(buf))
    } else {
        let mut buf = [0u8; 4];
        r.read_exact(&mut buf)?;
        Ok(u64::from(u32::from_be_bytes(buf)))
    }
}

/// Walks `udta` for `meta` -> { keys, ilst } and returns the value of the
/// `com.apple.quicktime.creationdate` key if present.
///
/// QuickTime metadata stores keys and values in two parallel boxes:
///   keys  : list of (namespace, key_name)
///   ilst  : list of items indexed by 1-based key id
fn parse_udta<R: Read + Seek>(
    r: &mut R,
    payload_len: u64,
) -> std::io::Result<Option<String>> {
    let start = r.stream_position()?;
    let mut pos = 0u64;
    while pos + 8 <= payload_len {
        r.seek(SeekFrom::Start(start + pos))?;
        let (size, kind, hdr_len) = match read_box_header(r)? {
            Some(h) => h,
            None => break,
        };
        if size < hdr_len {
            break;
        }
        if &kind == b"meta" {
            return parse_meta(r, size - hdr_len);
        }
        pos += size;
    }
    Ok(None)
}

fn parse_meta<R: Read + Seek>(r: &mut R, payload_len: u64) -> std::io::Result<Option<String>> {
    // The QuickTime variant of 'meta' has NO version/flags prefix, while the
    // ISOBMFF variant has 4 fullbox bytes. We try both interpretations: peek
    // 4 bytes; if they look like a fullbox header (version=0, flags=0) and
    // the next 4 bytes look like a fourcc, skip them. Otherwise rewind.
    let start = r.stream_position()?;
    let mut peek = [0u8; 8];
    if r.read(&mut peek)? < 8 {
        return Ok(None);
    }
    let body_start = if peek[0] == 0 && peek[1] == 0 && peek[2] == 0 && peek[3] == 0 {
        // looks like fullbox prefix; the next bytes peek[4..8] are the first
        // child's size. Keep position right after the 4-byte prefix.
        r.seek(SeekFrom::Start(start + 4))?;
        start + 4
    } else {
        r.seek(SeekFrom::Start(start))?;
        start
    };

    let body_len = payload_len - (body_start - start);

    // First pass: locate `keys` and `ilst` offsets.
    let mut keys_pos: Option<(u64, u64)> = None;
    let mut ilst_pos: Option<(u64, u64)> = None;
    let mut pos = 0u64;
    while pos + 8 <= body_len {
        r.seek(SeekFrom::Start(body_start + pos))?;
        let (size, kind, hdr_len) = match read_box_header(r)? {
            Some(h) => h,
            None => break,
        };
        if size < hdr_len {
            break;
        }
        if &kind == b"keys" {
            keys_pos = Some((body_start + pos + hdr_len, size - hdr_len));
        } else if &kind == b"ilst" {
            ilst_pos = Some((body_start + pos + hdr_len, size - hdr_len));
        }
        pos += size;
    }

    let (keys_off, keys_len) = match keys_pos {
        Some(v) => v,
        None => return Ok(None),
    };
    let (ilst_off, ilst_len) = match ilst_pos {
        Some(v) => v,
        None => return Ok(None),
    };

    let key_names = parse_keys(r, keys_off, keys_len)?;
    let target_idx = key_names
        .iter()
        .position(|n| n == "com.apple.quicktime.creationdate");
    let target_idx = match target_idx {
        Some(i) => i + 1, // ilst items are 1-indexed by key
        None => return Ok(None),
    };

    parse_ilst_string(r, ilst_off, ilst_len, target_idx as u32)
}

fn parse_keys<R: Read + Seek>(
    r: &mut R,
    off: u64,
    len: u64,
) -> std::io::Result<Vec<String>> {
    r.seek(SeekFrom::Start(off))?;
    // 4 bytes version+flags, 4 bytes entry count, then entries:
    //   4 bytes key size (incl. these 8 bytes), 4 bytes key namespace, key bytes
    let mut hdr = [0u8; 8];
    if r.read(&mut hdr)? < 8 {
        return Ok(vec![]);
    }
    let count = u32::from_be_bytes([hdr[4], hdr[5], hdr[6], hdr[7]]);
    let mut out = Vec::with_capacity(count as usize);
    let mut consumed = 8u64;
    for _ in 0..count {
        if consumed + 8 > len {
            break;
        }
        let mut k = [0u8; 8];
        r.read_exact(&mut k)?;
        let size = u32::from_be_bytes([k[0], k[1], k[2], k[3]]) as u64;
        if size < 8 || consumed + size > len {
            break;
        }
        let name_len = (size - 8) as usize;
        let mut name = vec![0u8; name_len];
        r.read_exact(&mut name)?;
        out.push(String::from_utf8_lossy(&name).into_owned());
        consumed += size;
    }
    Ok(out)
}

fn parse_ilst_string<R: Read + Seek>(
    r: &mut R,
    off: u64,
    len: u64,
    target_index: u32,
) -> std::io::Result<Option<String>> {
    r.seek(SeekFrom::Start(off))?;
    let mut pos = 0u64;
    while pos + 8 <= len {
        r.seek(SeekFrom::Start(off + pos))?;
        let mut hdr = [0u8; 8];
        if r.read(&mut hdr)? < 8 {
            break;
        }
        let item_size = u32::from_be_bytes([hdr[0], hdr[1], hdr[2], hdr[3]]) as u64;
        let item_index = u32::from_be_bytes([hdr[4], hdr[5], hdr[6], hdr[7]]);
        if item_size < 8 || pos + item_size > len {
            break;
        }
        if item_index == target_index {
            // Inside an item: child boxes; we want a `data` box.
            let inner_len = item_size - 8;
            let inner_start = off + pos + 8;
            let mut ipos = 0u64;
            while ipos + 8 <= inner_len {
                r.seek(SeekFrom::Start(inner_start + ipos))?;
                let mut ih = [0u8; 8];
                if r.read(&mut ih)? < 8 {
                    break;
                }
                let csize = u32::from_be_bytes([ih[0], ih[1], ih[2], ih[3]]) as u64;
                let ckind = [ih[4], ih[5], ih[6], ih[7]];
                if csize < 8 || ipos + csize > inner_len {
                    break;
                }
                if &ckind == b"data" {
                    // data box: 4 bytes type, 4 bytes locale, then payload.
                    if csize < 16 {
                        return Ok(None);
                    }
                    let payload_len = (csize - 16) as usize;
                    let mut skip = [0u8; 8];
                    r.read_exact(&mut skip)?;
                    let mut payload = vec![0u8; payload_len];
                    r.read_exact(&mut payload)?;
                    return Ok(Some(String::from_utf8_lossy(&payload).into_owned()));
                }
                ipos += csize;
            }
        }
        pos += item_size;
    }
    Ok(None)
}

/// Parse a QuickTime creationdate string (typically ISO-8601 with offset, e.g.
/// `2024-08-12T14:32:11-0700`).
fn parse_qt_date(s: &str) -> Option<DateTime<Utc>> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Some(dt.with_timezone(&Utc));
    }
    // QuickTime sometimes uses `-0700` (no colon).
    if let Ok(dt) = DateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%z") {
        return Some(dt.with_timezone(&Utc));
    }
    None
}
