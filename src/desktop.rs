//! Which Claude Code desktop sessions have something you have not read: the app's own sidebar list,
//! kept in its local storage (the `epitaxy-unread-v1` key, a Chromium LevelDB under `Local Storage/leveldb`).
//! That storage belongs to the app and is not documented, so this is lenient: it looks for the newest copy
//! of the list in the write-ahead log and the (Snappy-compressed) tables, and says nothing (None) when it
//! finds none.
use serde_json::Value;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

const MARK: &[u8] = br#"{"state":{"unreadIds":"#;

/// The desktop app's local storage folder.
pub fn storage() -> Option<PathBuf> {
    let home = PathBuf::from(std::env::var_os("HOME")?);
    let app = if cfg!(target_os = "macos") { home.join("Library/Application Support/Claude") } else { home.join(".config/Claude") };
    Some(app.join("Local Storage/leveldb"))
}

/// Every copy of the list in some bytes, in the order they appear: both its unread and its marked-unread ids.
fn lists_in(bytes: &[u8]) -> Vec<HashSet<String>> {
    let starts = bytes.windows(MARK.len()).enumerate().filter(|(_, w)| *w == MARK).map(|(i, _)| i);
    starts
        .filter_map(|i| {
            let v: Value = serde_json::Deserializer::from_slice(&bytes[i..]).into_iter::<Value>().next()?.ok()?;
            let ids = ["unreadIds", "explicitUnreadIds"].iter().flat_map(|k| v["state"][k].as_array().cloned().unwrap_or_default());
            Some(ids.filter_map(|id| id.as_str().map(str::to_string)).collect())
        })
        .collect()
}

fn varint(b: &[u8], at: &mut usize) -> Option<u64> {
    let mut n = 0u64;
    for shift in (0..64).step_by(7) {
        let byte = *b.get(*at)?;
        *at += 1;
        n |= u64::from(byte & 0x7f) << shift;
        if byte < 0x80 {
            return Some(n);
        }
    }
    None
}

/// The block of a LevelDB table that a handle (read from `handle` at `at`) points to; its trailer says
/// whether it is Snappy-compressed.
fn block(table: &[u8], handle: &[u8], at: &mut usize) -> Option<Vec<u8>> {
    let (offset, size) = (varint(handle, at)? as usize, varint(handle, at)? as usize);
    let raw = table.get(offset..offset.checked_add(size)?)?;
    match table.get(offset + size)? {
        0 => Some(raw.to_vec()),
        1 => snap::raw::Decoder::new().decompress_vec(raw).ok(),
        _ => None,
    }
}

/// A LevelDB table's data blocks, uncompressed, in key order; empty when it is not one.
fn table_blocks(table: &[u8]) -> Vec<Vec<u8>> {
    const MAGIC: u64 = 0xdb47_7524_8b80_fb57;
    let read = || -> Option<Vec<Vec<u8>>> {
        let footer = table.len().checked_sub(48)?;
        (u64::from_le_bytes(table[table.len() - 8..].try_into().ok()?) == MAGIC).then_some(())?;
        let mut at = footer;
        varint(table, &mut at)?;
        varint(table, &mut at)?;
        let index = block(table, table, &mut at)?;
        let restarts = u32::from_le_bytes(index.get(index.len().checked_sub(4)?..)?.try_into().ok()?) as usize;
        let end = index.len().checked_sub(4 + 4 * restarts)?;
        let (mut at, mut blocks) = (0, Vec::new());
        while at < end {
            let (_shared, unshared, len) = (varint(&index, &mut at)?, varint(&index, &mut at)? as usize, varint(&index, &mut at)? as usize);
            at += unshared;
            let mut handle = at;
            blocks.push(block(table, &index, &mut handle)?);
            at += len;
        }
        Some(blocks)
    };
    read().unwrap_or_default()
}

/// The newest copy of the list in one file: the last in a log, which only appends; the first in a table,
/// which keeps a key's versions newest first.
fn list_in(path: &Path) -> Option<HashSet<String>> {
    let bytes = std::fs::read(path).ok()?;
    if path.extension()? == "log" {
        return lists_in(&bytes).pop();
    }
    table_blocks(&bytes).iter().find_map(|b| lists_in(b).into_iter().next())
}

/// The ids (`local_…`) the app shows as unread; None when the list cannot be found.
pub fn unread(dir: &Path) -> Option<HashSet<String>> {
    let mut files: Vec<(u64, PathBuf)> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            let ext = p.extension()?.to_str()?;
            (ext == "log" || ext == "ldb").then_some(())?;
            Some((p.file_stem()?.to_str()?.parse().ok()?, p))
        })
        .collect();
    // LevelDB numbers its files as it makes them: the newest write is in the highest.
    files.sort_by(|a, b| b.0.cmp(&a.0));
    files.iter().find_map(|(_, p)| list_in(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A one-block LevelDB table holding `value`, Snappy-compressed as the app's are.
    fn table(value: &[u8]) -> Vec<u8> {
        fn put(out: &mut Vec<u8>, mut n: u64) {
            while n >= 0x80 {
                out.push((n as u8) | 0x80);
                n >>= 7;
            }
            out.push(n as u8);
        }
        let mut data = Vec::new();
        for n in [0, 3, value.len() as u64] {
            put(&mut data, n);
        }
        data.extend_from_slice(b"key");
        data.extend_from_slice(value);
        data.extend_from_slice(&[0, 0, 0, 0, 1, 0, 0, 0]);
        let data = snap::raw::Encoder::new().compress_vec(&data).unwrap();
        let mut out = data.clone();
        out.extend_from_slice(&[1, 0, 0, 0, 0]);
        let mut handle = Vec::new();
        put(&mut handle, 0);
        put(&mut handle, data.len() as u64);
        let mut index = Vec::new();
        for n in [0, 1, handle.len() as u64] {
            put(&mut index, n);
        }
        index.push(b'k');
        index.extend_from_slice(&handle);
        index.extend_from_slice(&[0, 0, 0, 0, 1, 0, 0, 0]);
        let index_at = out.len() as u64;
        out.extend_from_slice(&index);
        out.extend_from_slice(&[0, 0, 0, 0, 0]);
        let mut footer = Vec::new();
        put(&mut footer, index_at);
        put(&mut footer, 0);
        put(&mut footer, index_at);
        put(&mut footer, index.len() as u64);
        footer.resize(40, 0);
        footer.extend_from_slice(&0xdb47_7524_8b80_fb57u64.to_le_bytes());
        out.extend_from_slice(&footer);
        out
    }

    #[test]
    fn the_newest_whole_copy_in_the_log_wins() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("000007.ldb"), table(br#"{"state":{"unreadIds":["local_old"],"explicitUnreadIds":[]},"version":0}"#)).unwrap();
        let log = [&br#"x{"state":{"unreadIds":["local_a"],"explicitUnreadIds":["local_b"]},"version":0}"#[..], b"\x00\x01", &br#"{"state":{"unreadIds":["local_cut"#[..]].concat();
        std::fs::write(dir.path().join("000009.log"), log).unwrap();
        std::fs::write(dir.path().join("LOG"), br#"{"state":{"unreadIds":["local_not_a_table"]}}"#).unwrap();
        assert_eq!(unread(dir.path()).unwrap(), HashSet::from(["local_a".to_string(), "local_b".to_string()]));
    }

    #[test]
    fn a_compressed_table_answers_when_the_log_has_no_copy() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("000007.ldb"), table(br#"\x01{"state":{"unreadIds":["local_x"],"explicitUnreadIds":[]},"version":0}"#)).unwrap();
        std::fs::write(dir.path().join("000009.log"), b"other keys").unwrap();
        std::fs::write(dir.path().join("000010.ldb"), b"not a table").unwrap();
        assert_eq!(unread(dir.path()).unwrap(), HashSet::from(["local_x".to_string()]));
        assert!(unread(&dir.path().join("missing")).is_none());
    }
}
