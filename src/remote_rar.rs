//! Reads pages out of a `.cbr` on a server without downloading all of it.
//!
//! A RAR has no table of contents: each file's header sits right before
//! its data. So opening walks the headers, one small range request per
//! entry that hops over the data, and remembers where every page is (the
//! index is kept in the cache, so a chapter is only walked once). A page is
//! then one range request for its header and data, which is unpacked on
//! its own by wrapping it in a one-file archive for libarchive.
//!
//! That only works when each file was packed on its own. A "solid"
//! archive compresses every file as one stream, so a page can't be had
//! without everything before it; [`RemoteRar::solid`] says so, and such
//! files have to be downloaded whole. Encrypted and multi-volume archives
//! aren't supported here either.

use std::path::Path;
use std::sync::Arc;

use crate::remote_zip::RangeSource;

const RAR5_SIG: &[u8] = b"Rar!\x1a\x07\x01\x00";
const RAR4_SIG: &[u8] = b"Rar!\x1a\x07\x00";
/// RAR 4's end-of-archive block, always the same bytes.
const RAR4_END: &[u8] = &[0xC4, 0x3D, 0x7B, 0x00, 0x40, 0x07, 0x00];
/// Enough for almost any header in one request.
const HEAD_FETCH: u64 = 4096;

/// One file in the archive.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Entry {
    pub name: String,
    header_offset: u64,
    header_len: u64,
    data_size: u64,
    /// Stored without compression: the data is the file.
    stored: bool,
}

/// Where an archive's pages are, as saved in the cache.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct Index {
    /// The archive's size, to notice when it changed on the server.
    len: u64,
    version: u8,
    /// RAR 4's main header, copied into each one-file archive.
    main: Vec<u8>,
    solid: bool,
    entries: Vec<Entry>,
}

pub struct RemoteRar {
    source: Box<dyn RangeSource>,
    index: Index,
}

impl std::fmt::Debug for RemoteRar {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemoteRar").field("entries", &self.index.entries.len()).field("len", &self.index.len).finish()
    }
}

impl RemoteRar {
    /// Reads the archive's headers (or the index saved at `cache` from an
    /// earlier time) and keeps the image entries in reading order.
    pub fn open(source: Box<dyn RangeSource>, cache: Option<&Path>) -> Result<Arc<Self>, String> {
        let (_, len) = source.tail(1)?;
        let saved = cache
            .and_then(|c| std::fs::read(c).ok())
            .and_then(|b| serde_json::from_slice::<Index>(&b).ok())
            .filter(|i| i.len == len);
        let index = match saved {
            Some(index) => index,
            None => {
                let mut index = walk(source.as_ref(), len)?;
                index.entries.retain(|e| crate::comic::is_image_name(&e.name));
                index
                    .entries
                    .sort_by(|a, b| crate::comic::natural_key(&a.name).cmp(&crate::comic::natural_key(&b.name)));
                if let Some(c) = cache {
                    if let Some(dir) = c.parent() {
                        let _ = std::fs::create_dir_all(dir);
                    }
                    if let Ok(json) = serde_json::to_vec(&index) {
                        let _ = std::fs::write(c, json);
                    }
                }
                index
            }
        };
        Ok(Arc::new(RemoteRar { source, index }))
    }

    pub fn entries(&self) -> &[Entry] {
        &self.index.entries
    }

    /// The archive's size in bytes.
    pub fn len(&self) -> u64 {
        self.index.len
    }

    /// Packed as one stream: pages can't be read on their own.
    pub fn solid(&self) -> bool {
        self.index.solid
    }

    /// The unpacked bytes of entry `index`.
    pub fn read(&self, index: usize) -> Result<Vec<u8>, String> {
        let e = self.index.entries.get(index).ok_or_else(|| format!("no page {index}"))?;
        let bytes = self.source.range(e.header_offset, e.header_offset + e.header_len + e.data_size)?;
        if (bytes.len() as u64) < e.header_len + e.data_size {
            return Err(format!("{}: the server sent less than asked", e.name));
        }
        if e.stored {
            return Ok(bytes[e.header_len as usize..].to_vec());
        }
        let archive = one_file_archive(&self.index, &bytes);
        unpack_first(archive).map_err(|err| format!("{}: {err}", e.name))
    }
}

/// A complete archive holding just one file's header and data.
fn one_file_archive(index: &Index, file: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(file.len() + 64);
    if index.version == 5 {
        out.extend_from_slice(RAR5_SIG);
        out.extend_from_slice(&rar5_header(1, &vint(0)));
        out.extend_from_slice(file);
        out.extend_from_slice(&rar5_header(5, &vint(0)));
    } else {
        out.extend_from_slice(RAR4_SIG);
        out.extend_from_slice(&index.main);
        out.extend_from_slice(file);
        out.extend_from_slice(RAR4_END);
    }
    out
}

fn unpack_first(archive: Vec<u8>) -> Result<Vec<u8>, String> {
    use compress_tools::{ArchiveContents, ArchiveIterator};

    let iter = ArchiveIterator::from_read(std::io::Cursor::new(archive)).map_err(|e| e.to_string())?;
    let mut data = Vec::new();
    for content in iter {
        match content {
            ArchiveContents::StartOfEntry(..) => {}
            ArchiveContents::DataChunk(chunk) => data.extend_from_slice(&chunk),
            ArchiveContents::EndOfEntry => return Ok(data),
            ArchiveContents::Err(e) => return Err(e.to_string()),
        }
    }
    Err("empty archive".into())
}

/// Walks every header from the start of the file.
fn walk(source: &dyn RangeSource, len: u64) -> Result<Index, String> {
    let start = source.range(0, HEAD_FETCH.min(len))?;
    if start.starts_with(RAR5_SIG) {
        walk5(source, len)
    } else if start.starts_with(RAR4_SIG) {
        walk4(source, len)
    } else {
        Err("not a RAR archive".into())
    }
}

/// The bytes `at..at+n` (fewer at the end of the file).
fn fetch(source: &dyn RangeSource, len: u64, at: u64, n: u64) -> Result<Vec<u8>, String> {
    source.range(at, (at + n).min(len))
}

/// A RAR 5 variable-length integer: (value, bytes used).
fn read_vint(b: &[u8], at: usize) -> Option<(u64, usize)> {
    let mut value = 0u64;
    for i in 0..10 {
        let byte = *b.get(at + i)?;
        value |= u64::from(byte & 0x7F) << (7 * i);
        if byte & 0x80 == 0 {
            return Some((value, i + 1));
        }
    }
    None
}

fn vint(mut v: u64) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let byte = (v & 0x7F) as u8;
        v >>= 7;
        if v == 0 {
            out.push(byte);
            return out;
        }
        out.push(byte | 0x80);
    }
}

/// A RAR 5 header of `kind` with no flags: CRC32, size, then the fields.
fn rar5_header(kind: u64, fields: &[u8]) -> Vec<u8> {
    let mut body = vint(kind);
    body.extend_from_slice(&vint(0));
    body.extend_from_slice(fields);
    let mut sized = vint(body.len() as u64);
    sized.extend_from_slice(&body);
    let mut out = crc32fast::hash(&sized).to_le_bytes().to_vec();
    out.extend_from_slice(&sized);
    out
}

/// Reads RAR 5 fields one after another.
struct Cursor<'a> {
    b: &'a [u8],
    p: usize,
}

impl Cursor<'_> {
    fn vint(&mut self) -> Result<u64, String> {
        let (v, used) = read_vint(self.b, self.p).ok_or("damaged RAR header (field)")?;
        self.p += used;
        Ok(v)
    }
}

fn walk5(source: &dyn RangeSource, len: u64) -> Result<Index, String> {
    let bad = |what: &str| format!("damaged RAR header ({what})");
    let mut index = Index { len, version: 5, main: Vec::new(), solid: false, entries: Vec::new() };
    let mut at = RAR5_SIG.len() as u64;
    while at + 7 <= len {
        let mut b = fetch(source, len, at, HEAD_FETCH)?;
        let (size, n) = read_vint(&b, 4).ok_or_else(|| bad("size"))?;
        let header_len = 4 + n as u64 + size;
        if (b.len() as u64) < header_len {
            b = fetch(source, len, at, header_len)?;
        }
        let b = b.get(..header_len as usize).ok_or_else(|| bad("short"))?;
        let mut c = Cursor { b, p: 4 + n };
        let kind = c.vint()?;
        let flags = c.vint()?;
        let extra_size = if flags & 0x0001 != 0 { c.vint()? } else { 0 };
        let data_size = if flags & 0x0002 != 0 { c.vint()? } else { 0 };
        match kind {
            1 => {
                let archive_flags = c.vint()?;
                if archive_flags & 0x0001 != 0 {
                    return Err("multi-volume RAR archives aren't supported".into());
                }
                index.solid |= archive_flags & 0x0004 != 0;
            }
            2 => {
                let file_flags = c.vint()?;
                let _unpacked = c.vint()?;
                let _attributes = c.vint()?;
                if file_flags & 0x0002 != 0 {
                    c.p += 4;
                }
                if file_flags & 0x0004 != 0 {
                    c.p += 4;
                }
                let compression = c.vint()?;
                let _host = c.vint()?;
                let name_len = c.vint()? as usize;
                let name = b.get(c.p..c.p + name_len).ok_or_else(|| bad("name"))?;
                let name = String::from_utf8_lossy(name).into_owned();
                // Extra records: type 1 is encryption.
                let extra = b.get(header_len.saturating_sub(extra_size) as usize..).unwrap_or_default();
                let mut x = 0;
                let mut encrypted = false;
                while let Some((rec_size, used)) = read_vint(extra, x) {
                    if rec_size == 0 {
                        break;
                    }
                    encrypted |= read_vint(extra, x + used).is_some_and(|(t, _)| t == 1);
                    x += used + rec_size as usize;
                }
                index.solid |= compression & 0x40 != 0;
                if file_flags & 0x0001 == 0 && !encrypted {
                    index.entries.push(Entry {
                        name,
                        header_offset: at,
                        header_len,
                        data_size,
                        stored: (compression >> 7) & 7 == 0,
                    });
                }
            }
            4 => return Err("encrypted RAR archives aren't supported".into()),
            5 => break,
            _ => {}
        }
        at += header_len + data_size;
    }
    Ok(index)
}

fn walk4(source: &dyn RangeSource, len: u64) -> Result<Index, String> {
    let bad = |what: &str| format!("damaged RAR header ({what})");
    let u16_at = |b: &[u8], i: usize| b.get(i..i + 2).map(|s| u16::from_le_bytes([s[0], s[1]]));
    let u32_at = |b: &[u8], i: usize| b.get(i..i + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]));
    let mut index = Index { len, version: 4, main: Vec::new(), solid: false, entries: Vec::new() };
    let mut at = RAR4_SIG.len() as u64;
    while at + 7 <= len {
        let mut b = fetch(source, len, at, HEAD_FETCH)?;
        let kind = *b.get(2).ok_or_else(|| bad("type"))?;
        let flags = u16_at(&b, 3).ok_or_else(|| bad("flags"))?;
        let header_len = u64::from(u16_at(&b, 5).ok_or_else(|| bad("size"))?);
        if header_len < 7 {
            return Err(bad("size"));
        }
        if (b.len() as u64) < header_len {
            b = fetch(source, len, at, header_len)?;
        }
        let b = b.get(..header_len as usize).ok_or_else(|| bad("short"))?;
        let mut data_size = if flags & 0x8000 != 0 { u64::from(u32_at(b, 7).unwrap_or(0)) } else { 0 };
        match kind {
            0x73 => {
                if flags & 0x0001 != 0 {
                    return Err("multi-volume RAR archives aren't supported".into());
                }
                if flags & 0x0080 != 0 {
                    return Err("encrypted RAR archives aren't supported".into());
                }
                index.solid |= flags & 0x0008 != 0;
                index.main = b.to_vec();
            }
            0x74 => {
                let large = flags & 0x0100 != 0;
                if large {
                    data_size |= u64::from(u32_at(b, 32).ok_or_else(|| bad("size"))?) << 32;
                }
                let method = *b.get(25).ok_or_else(|| bad("method"))?;
                let name_len = usize::from(u16_at(b, 26).ok_or_else(|| bad("name"))?);
                let name_at = if large { 40 } else { 32 };
                let raw = b.get(name_at..name_at + name_len).ok_or_else(|| bad("name"))?;
                // Unicode names follow the plain one after a zero byte.
                let raw = raw.split(|&c| c == 0).next().unwrap_or(raw);
                let name = String::from_utf8_lossy(raw).replace('\\', "/");
                index.solid |= flags & 0x0010 != 0;
                let directory = flags & 0x00E0 == 0x00E0;
                let encrypted = flags & 0x0004 != 0;
                if !directory && !encrypted {
                    index.entries.push(Entry { name, header_offset: at, header_len, data_size, stored: method == 0x30 });
                }
            }
            0x7B => break,
            _ => {}
        }
        at += header_len + data_size;
    }
    Ok(index)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A file or buffer standing in for the server, counting requests.
    struct Mem(Vec<u8>, Arc<AtomicUsize>);

    impl RangeSource for Mem {
        fn tail(&self, n: u64) -> Result<(Vec<u8>, u64), String> {
            self.1.fetch_add(1, Ordering::SeqCst);
            let start = self.0.len().saturating_sub(n as usize);
            Ok((self.0[start..].to_vec(), self.0.len() as u64))
        }
        fn range(&self, start: u64, end: u64) -> Result<Vec<u8>, String> {
            self.1.fetch_add(1, Ordering::SeqCst);
            Ok(self.0[start as usize..(end as usize).min(self.0.len())].to_vec())
        }
    }

    /// A RAR 5 archive of stored (uncompressed) files, built by hand.
    fn build_rar5(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut out = RAR5_SIG.to_vec();
        out.extend_from_slice(&rar5_header(1, &vint(0)));
        for (name, data) in files {
            // Type 2, flags: data area present.
            let mut body = vint(2);
            body.extend_from_slice(&vint(0x0002));
            body.extend_from_slice(&vint(data.len() as u64));
            body.extend_from_slice(&vint(0x0004)); // file flags: CRC present
            body.extend_from_slice(&vint(data.len() as u64));
            body.extend_from_slice(&vint(0x20)); // attributes
            body.extend_from_slice(&crc32fast::hash(data).to_le_bytes());
            body.extend_from_slice(&vint(0)); // compression: version 0, store
            body.extend_from_slice(&vint(0)); // host OS: Windows
            body.extend_from_slice(&vint(name.len() as u64));
            body.extend_from_slice(name.as_bytes());
            let mut sized = vint(body.len() as u64);
            sized.extend_from_slice(&body);
            out.extend_from_slice(&crc32fast::hash(&sized).to_le_bytes());
            out.extend_from_slice(&sized);
            out.extend_from_slice(data);
        }
        out.extend_from_slice(&rar5_header(5, &vint(0)));
        out
    }

    #[test]
    fn vints_round_trip() {
        for v in [0, 1, 127, 128, 300, 1 << 40] {
            let b = vint(v);
            assert_eq!(read_vint(&b, 0), Some((v, b.len())));
        }
    }

    #[test]
    fn reads_pages_of_a_rar5() {
        let pages: Vec<(String, Vec<u8>)> =
            (1..=3).map(|i| (format!("p{i}.jpg"), vec![i as u8; 10_000 * i])).collect();
        let mut files: Vec<(&str, &[u8])> = pages.iter().map(|(n, d)| (n.as_str(), d.as_slice())).collect();
        files.insert(1, ("info.txt", b"not a page"));
        files.swap(0, 3); // stored out of order
        let archive = build_rar5(&files);
        let requests = Arc::new(AtomicUsize::new(0));
        let rar = RemoteRar::open(Box::new(Mem(archive.clone(), requests.clone())), None).unwrap();
        assert!(!rar.solid());
        let names: Vec<&str> = rar.entries().iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["p1.jpg", "p2.jpg", "p3.jpg"]);
        assert_eq!(rar.read(1).unwrap(), pages[1].1);
        // The same page through libarchive, as compressed pages are read.
        let e = &rar.entries()[2];
        let bytes = &archive[e.header_offset as usize..(e.header_offset + e.header_len + e.data_size) as usize];
        assert_eq!(unpack_first(one_file_archive(&rar.index, bytes)).unwrap(), pages[2].1);

        // The index is saved and reused without walking again.
        let cache = std::env::temp_dir().join(format!("nebula-rar-index-{}.json", std::process::id()));
        RemoteRar::open(Box::new(Mem(archive.clone(), requests.clone())), Some(&cache)).unwrap();
        let before = requests.load(Ordering::SeqCst);
        let again = RemoteRar::open(Box::new(Mem(archive, requests.clone())), Some(&cache)).unwrap();
        assert_eq!(requests.load(Ordering::SeqCst) - before, 1);
        assert_eq!(again.entries().len(), 3);
        std::fs::remove_file(cache).unwrap();
    }

    /// Checks a real CBR: `NEBULA_RAR_FILE=path cargo test
    /// reads_a_real_rar -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn reads_a_real_rar() {
        let path = std::env::var("NEBULA_RAR_FILE").expect("set NEBULA_RAR_FILE");
        let data = std::fs::read(&path).unwrap();
        let requests = Arc::new(AtomicUsize::new(0));
        let t = std::time::Instant::now();
        let rar = RemoteRar::open(Box::new(Mem(data, requests.clone())), None).unwrap();
        println!(
            "{} pages, solid {}, {} requests, {:?}",
            rar.entries().len(),
            rar.solid(),
            requests.load(Ordering::SeqCst),
            t.elapsed()
        );
        let local = crate::comic::collect_sources(Path::new(&path)).unwrap().0;
        assert_eq!(local.len(), rar.entries().len());
        for i in [0, rar.entries().len() / 2, rar.entries().len() - 1] {
            let remote = rar.read(i).unwrap();
            let crate::comic::PageSource::Rar { archive, entry } = &local[i] else { panic!() };
            assert_eq!(entry, &rar.entries()[i].name);
            let whole = std::process::Command::new("bsdtar").arg("-xOf").arg(archive).arg(entry).output().unwrap().stdout;
            let first_diff = remote.iter().zip(&whole).position(|(a, b)| a != b);
            assert!(remote == whole, "page {i}: {} vs {} bytes, first difference at {first_diff:?}", remote.len(), whole.len());
        }
        println!("pages match");
    }
}
