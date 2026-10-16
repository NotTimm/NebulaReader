//! Reads pages out of a `.cbz` on a server without downloading it.
//!
//! A zip keeps its table of contents (the central directory) at the end of
//! the file, and each entry's bytes at a known offset. So opening costs one
//! HTTP range request for the last 64 KB (more only for huge archives), and
//! each page costs one request for exactly its own bytes. The reader's
//! normal read-ahead fetches the next pages while the current one is shown.

use std::io::Read;
use std::sync::Arc;

/// Something that can serve byte ranges of one file: an HTTP URL that
/// supports `Range`, or (in tests) a buffer.
pub trait RangeSource: Send + Sync {
    /// The last `n` bytes, plus the file's total length.
    fn tail(&self, n: u64) -> Result<(Vec<u8>, u64), String>;
    /// Bytes `start..end` (end exclusive).
    fn range(&self, start: u64, end: u64) -> Result<Vec<u8>, String>;
}

/// One page inside the archive.
#[derive(Clone, Debug)]
pub struct Entry {
    pub name: String,
    method: u16,
    compressed_size: u64,
    local_header_offset: u64,
}

/// An opened remote archive: its image entries in reading order.
pub struct RemoteZip {
    source: Box<dyn RangeSource>,
    pub entries: Vec<Entry>,
    len: u64,
}

impl std::fmt::Debug for RemoteZip {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemoteZip").field("entries", &self.entries.len()).field("len", &self.len).finish()
    }
}

const TAIL: u64 = 64 * 1024;
const EOCD_SIG: u32 = 0x0605_4b50;
const ZIP64_LOCATOR_SIG: u32 = 0x0706_4b50;
const ZIP64_EOCD_SIG: u32 = 0x0606_4b50;
const CDIR_SIG: u32 = 0x0201_4b50;
const LOCAL_SIG: u32 = 0x0403_4b50;

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    b.get(at..at + 2).map(|s| u16::from_le_bytes([s[0], s[1]]))
}
fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    b.get(at..at + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}
fn u64_at(b: &[u8], at: usize) -> Option<u64> {
    b.get(at..at + 8).map(|s| u64::from_le_bytes(s.try_into().unwrap()))
}

impl RemoteZip {
    /// Reads the archive's table of contents and keeps the image entries,
    /// sorted the same way local archives are.
    pub fn open(source: Box<dyn RangeSource>) -> Result<Arc<Self>, String> {
        let (tail, len) = source.tail(TAIL)?;
        let tail_start = len - tail.len() as u64;
        let corrupt = || "not a valid zip (central directory not found)".to_string();

        // End of central directory record: the last occurrence of its
        // signature (a trailing comment can follow it).
        let eocd = (0..tail.len().saturating_sub(21))
            .rev()
            .find(|&i| u32_at(&tail, i) == Some(EOCD_SIG))
            .ok_or_else(corrupt)?;
        let mut cd_size = u64::from(u32_at(&tail, eocd + 12).ok_or_else(corrupt)?);
        let mut cd_offset = u64::from(u32_at(&tail, eocd + 16).ok_or_else(corrupt)?);

        // Zip64: the real sizes live in a separate record found via a locator.
        if cd_size == 0xFFFF_FFFF || cd_offset == 0xFFFF_FFFF {
            let loc = eocd.checked_sub(20).filter(|&l| u32_at(&tail, l) == Some(ZIP64_LOCATOR_SIG));
            let rec_offset = loc.and_then(|l| u64_at(&tail, l + 8)).ok_or_else(corrupt)?;
            let rec = if rec_offset >= tail_start {
                tail[(rec_offset - tail_start) as usize..].to_vec()
            } else {
                source.range(rec_offset, rec_offset + 56)?
            };
            if u32_at(&rec, 0) != Some(ZIP64_EOCD_SIG) {
                return Err(corrupt());
            }
            cd_size = u64_at(&rec, 40).ok_or_else(corrupt)?;
            cd_offset = u64_at(&rec, 48).ok_or_else(corrupt)?;
        }

        let cd = if cd_offset >= tail_start {
            let from = (cd_offset - tail_start) as usize;
            tail.get(from..from + cd_size as usize).ok_or_else(corrupt)?.to_vec()
        } else {
            source.range(cd_offset, cd_offset + cd_size)?
        };

        let mut entries = parse_central_directory(&cd).ok_or_else(corrupt)?;
        entries.retain(|e| crate::comic::is_image_name(&e.name) && !e.name.ends_with('/'));
        entries.sort_by(|a, b| crate::comic::natural_key(&a.name).cmp(&crate::comic::natural_key(&b.name)));
        Ok(Arc::new(RemoteZip { source, entries, len }))
    }

    /// The (decompressed) bytes of entry `index`.
    pub fn read(&self, index: usize) -> Result<Vec<u8>, String> {
        let e = self.entries.get(index).ok_or_else(|| format!("no page {index}"))?;
        // Fetch the local header and the data in one request; the local
        // header's "extra" field is usually small, so allow some slack.
        const SLACK: u64 = 30 + 512;
        let want = (SLACK + e.name.len() as u64 + e.compressed_size).min(self.len - e.local_header_offset);
        let mut buf = self.source.range(e.local_header_offset, e.local_header_offset + want)?;
        if u32_at(&buf, 0) != Some(LOCAL_SIG) {
            return Err(format!("{}: bad local header", e.name));
        }
        let name_len = u16_at(&buf, 26).unwrap_or(0) as usize;
        let extra_len = u16_at(&buf, 28).unwrap_or(0) as usize;
        let data_start = 30 + name_len + extra_len;
        let data_end = data_start + e.compressed_size as usize;
        if buf.len() < data_end {
            // Unusually large extra field: fetch the rest.
            let more = self.source.range(e.local_header_offset + buf.len() as u64, e.local_header_offset + data_end as u64)?;
            buf.extend_from_slice(&more);
        }
        let data = &buf[data_start..data_end];
        match e.method {
            0 => Ok(data.to_vec()),
            8 => {
                let mut out = Vec::new();
                flate2::read::DeflateDecoder::new(data)
                    .read_to_end(&mut out)
                    .map_err(|err| format!("{}: {err}", e.name))?;
                Ok(out)
            }
            m => Err(format!("{}: unsupported compression method {m}", e.name)),
        }
    }
}

fn parse_central_directory(cd: &[u8]) -> Option<Vec<Entry>> {
    let mut entries = Vec::new();
    let mut at = 0;
    while u32_at(cd, at) == Some(CDIR_SIG) {
        let flags = u16_at(cd, at + 8)?;
        let method = u16_at(cd, at + 10)?;
        let mut compressed = u64::from(u32_at(cd, at + 20)?);
        let uncompressed = u32_at(cd, at + 24)?;
        let name_len = u16_at(cd, at + 28)? as usize;
        let extra_len = u16_at(cd, at + 30)? as usize;
        let comment_len = u16_at(cd, at + 32)? as usize;
        let mut offset = u64::from(u32_at(cd, at + 42)?);
        let name_bytes = cd.get(at + 46..at + 46 + name_len)?;
        let name = String::from_utf8_lossy(name_bytes).into_owned();

        // Zip64 extra field (id 1) carries whichever of these overflowed,
        // in this order: uncompressed size, compressed size, offset.
        let extra = cd.get(at + 46 + name_len..at + 46 + name_len + extra_len)?;
        let mut x = 0;
        while x + 4 <= extra.len() {
            let id = u16_at(extra, x)?;
            let size = u16_at(extra, x + 2)? as usize;
            if id == 1 {
                let mut f = x + 4;
                if uncompressed == 0xFFFF_FFFF {
                    f += 8;
                }
                if compressed == 0xFFFF_FFFF {
                    compressed = u64_at(extra, f)?;
                    f += 8;
                }
                if offset == 0xFFFF_FFFF {
                    offset = u64_at(extra, f)?;
                }
            }
            x += 4 + size;
        }

        // Encrypted entries can't be shown; skip them.
        if flags & 1 == 0 {
            entries.push(Entry { name, method, compressed_size: compressed, local_header_offset: offset });
        }
        at += 46 + name_len + extra_len + comment_len;
    }
    Some(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// An in-memory "server" that counts requests.
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

    fn build_zip(files: &[(&str, Vec<u8>, zip::CompressionMethod)]) -> Vec<u8> {
        let mut buf = std::io::Cursor::new(Vec::new());
        let mut w = zip::ZipWriter::new(&mut buf);
        for (name, data, method) in files {
            let opts = zip::write::SimpleFileOptions::default().compression_method(*method);
            if name.ends_with('/') {
                w.add_directory(name.trim_end_matches('/'), opts).unwrap();
            } else {
                w.start_file(*name, opts).unwrap();
                w.write_all(data).unwrap();
            }
        }
        w.finish().unwrap();
        buf.into_inner()
    }

    #[test]
    fn lists_and_reads_pages_with_one_request_each() {
        let page = |n: u8| vec![n; 5000 + n as usize];
        let zip = build_zip(&[
            ("ComicInfo.xml", b"<ComicInfo/>".to_vec(), zip::CompressionMethod::Deflated),
            ("pages/", vec![], zip::CompressionMethod::Stored),
            ("pages/p10.jpg", page(10), zip::CompressionMethod::Stored),
            ("pages/p2.jpg", page(2), zip::CompressionMethod::Deflated),
            ("pages/p1.png", page(1), zip::CompressionMethod::Stored),
        ]);
        let requests = Arc::new(AtomicUsize::new(0));
        let z = RemoteZip::open(Box::new(Mem(zip, requests.clone()))).unwrap();

        // Images only, in natural order (p1, p2, p10).
        let names: Vec<_> = z.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["pages/p1.png", "pages/p2.jpg", "pages/p10.jpg"]);
        // A small archive's directory comes with the first tail read.
        assert_eq!(requests.load(Ordering::SeqCst), 1);

        assert_eq!(z.read(0).unwrap(), page(1));
        assert_eq!(z.read(1).unwrap(), page(2)); // deflated
        assert_eq!(z.read(2).unwrap(), page(10));
        assert_eq!(requests.load(Ordering::SeqCst), 4);
    }

    #[test]
    fn directory_beyond_the_tail_is_fetched_separately() {
        // 2000 entries make the central directory bigger than the 64 KB tail.
        let files: Vec<_> = (0..2000)
            .map(|i| (format!("a-very-long-page-name-to-grow-the-directory-{i:05}.jpg"), vec![i as u8; 3]))
            .collect();
        let refs: Vec<_> = files.iter().map(|(n, d)| (n.as_str(), d.clone(), zip::CompressionMethod::Stored)).collect();
        let zip = build_zip(&refs);
        let requests = Arc::new(AtomicUsize::new(0));
        let z = RemoteZip::open(Box::new(Mem(zip, requests.clone()))).unwrap();
        assert_eq!(z.entries.len(), 2000);
        assert_eq!(requests.load(Ordering::SeqCst), 2);
        assert_eq!(z.read(1999).unwrap(), vec![(1999 % 256) as u8; 3]);
    }

    #[test]
    fn rejects_non_zips() {
        let requests = Arc::new(AtomicUsize::new(0));
        assert!(RemoteZip::open(Box::new(Mem(b"Rar!\x1a\x07\x00 not a zip".to_vec(), requests))).is_err());
    }
}
