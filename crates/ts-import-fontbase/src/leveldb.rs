//! Minimal read-only LevelDB reader: write-ahead `.log` files and `.ldb` tables.
//! Returns the newest value of every key; no locking, so it works while FontBase runs.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result, bail, ensure};

const BLOCK: usize = 32768;
const TABLE_MAGIC: u64 = 0xdb4775248b80fb57;

/// Newest value per key; deleted keys are absent.
pub fn read_dir(dir: &Path) -> Result<HashMap<Vec<u8>, Vec<u8>>> {
    let mut entries: HashMap<Vec<u8>, (u64, Option<Vec<u8>>)> = HashMap::new();
    let mut put = |key: Vec<u8>, seq: u64, value: Option<Vec<u8>>| match entries.get(&key) {
        Some((s, _)) if *s >= seq => {}
        _ => {
            entries.insert(key, (seq, value));
        }
    };
    for entry in std::fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let path = entry?.path();
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or_default();
        if ext != "log" && ext != "ldb" && ext != "sst" {
            continue;
        }
        let data = std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
        let result = if ext == "log" { read_log(&data, &mut put) } else { read_table(&data, &mut put) };
        result.with_context(|| format!("parsing {}", path.display()))?;
    }
    Ok(entries.into_iter().filter_map(|(k, (_, v))| Some((k, v?))).collect())
}

pub fn varint(b: &[u8], i: &mut usize) -> Result<u64> {
    let mut v = 0u64;
    for shift in (0..64).step_by(7) {
        let byte = *b.get(*i).context("truncated varint")?;
        *i += 1;
        v |= u64::from(byte & 0x7f) << shift;
        if byte < 0x80 {
            return Ok(v);
        }
    }
    bail!("varint too long")
}

fn slice<'a>(b: &'a [u8], i: &mut usize, n: usize) -> Result<&'a [u8]> {
    let s = b.get(*i..*i + n).context("truncated record")?;
    *i += n;
    Ok(s)
}

/// Log: 32 KiB blocks of fragments (checksum u32, length u16, type u8) that join into write batches.
fn read_log(data: &[u8], put: &mut impl FnMut(Vec<u8>, u64, Option<Vec<u8>>)) -> Result<()> {
    let mut record = Vec::new();
    for block in data.chunks(BLOCK) {
        let mut i = 0;
        while i + 7 <= block.len() {
            let len = u16::from_le_bytes([block[i + 4], block[i + 5]]) as usize;
            let kind = block[i + 6];
            if kind == 0 && len == 0 {
                break;
            }
            let Some(payload) = block.get(i + 7..i + 7 + len) else { break };
            i += 7 + len;
            match kind {
                1 => batch(payload, put)?,
                2 => record = payload.to_vec(),
                3 => record.extend_from_slice(payload),
                4 => {
                    record.extend_from_slice(payload);
                    batch(&record, put)?;
                    record.clear();
                }
                _ => {}
            }
        }
    }
    Ok(())
}

/// A write batch: sequence u64, count u32, then put (1) / delete (0) entries.
fn batch(b: &[u8], put: &mut impl FnMut(Vec<u8>, u64, Option<Vec<u8>>)) -> Result<()> {
    ensure!(b.len() >= 12, "short batch");
    let seq = u64::from_le_bytes(b[..8].try_into()?);
    let count = u32::from_le_bytes(b[8..12].try_into()?);
    let mut i = 12;
    for n in 0..count as u64 {
        let tag = *b.get(i).context("truncated batch")?;
        i += 1;
        let klen = varint(b, &mut i)? as usize;
        let key = slice(b, &mut i, klen)?.to_vec();
        if tag == 1 {
            let vlen = varint(b, &mut i)? as usize;
            put(key, seq + n, Some(slice(b, &mut i, vlen)?.to_vec()));
        } else {
            put(key, seq + n, None);
        }
    }
    Ok(())
}

/// Table: footer → index block → data blocks. Keys carry an 8-byte (sequence << 8 | type) trailer.
fn read_table(data: &[u8], put: &mut impl FnMut(Vec<u8>, u64, Option<Vec<u8>>)) -> Result<()> {
    ensure!(data.len() >= 48, "short table");
    let footer = &data[data.len() - 48..];
    ensure!(u64::from_le_bytes(footer[40..48].try_into()?) == TABLE_MAGIC, "not a LevelDB table");
    let mut i = 0;
    let _meta = (varint(footer, &mut i)?, varint(footer, &mut i)?);
    let (index_off, index_len) = (varint(footer, &mut i)?, varint(footer, &mut i)?);
    let index = block_contents(data, index_off, index_len)?;
    for (_, handle) in block_entries(&index)? {
        let mut j = 0;
        let (off, len) = (varint(&handle, &mut j)?, varint(&handle, &mut j)?);
        for (ikey, value) in block_entries(&block_contents(data, off, len)?)? {
            ensure!(ikey.len() >= 8, "short internal key");
            let (user, trailer) = ikey.split_at(ikey.len() - 8);
            let trailer = u64::from_le_bytes(trailer.try_into()?);
            let value = (trailer & 0xff == 1).then_some(value);
            put(user.to_vec(), trailer >> 8, value);
        }
    }
    Ok(())
}

fn block_contents(data: &[u8], off: u64, len: u64) -> Result<Vec<u8>> {
    let (off, len) = (off as usize, len as usize);
    let raw = data.get(off..off + len).context("block out of range")?;
    match data.get(off + len) {
        Some(0) => Ok(raw.to_vec()),
        Some(1) => Ok(snap::raw::Decoder::new().decompress_vec(raw)?),
        other => bail!("unknown block compression {other:?}"),
    }
}

/// Prefix-compressed entries; the trailer holds restart offsets and their count.
fn block_entries(b: &[u8]) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
    ensure!(b.len() >= 4, "short block");
    let restarts = u32::from_le_bytes(b[b.len() - 4..].try_into()?) as usize;
    let end = b.len().checked_sub(4 + 4 * restarts).context("bad restart count")?;
    let mut out = Vec::new();
    let mut key: Vec<u8> = Vec::new();
    let mut i = 0;
    while i < end {
        let shared = varint(b, &mut i)? as usize;
        let unshared = varint(b, &mut i)? as usize;
        let vlen = varint(b, &mut i)? as usize;
        key.truncate(shared);
        key.extend_from_slice(slice(b, &mut i, unshared)?);
        out.push((key.clone(), slice(b, &mut i, vlen)?.to_vec()));
    }
    Ok(out)
}
