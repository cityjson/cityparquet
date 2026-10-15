//! Byte searches over a whole document, which let the up-front pass be
//! skipped for a document that certainly has nothing it reads.

use std::fs::File;
use std::io::Read;
use std::path::Path;

use crate::xml::is_ade;
use crate::{Error, Result};

/// Whether a file starts like a document in an encoding that is not
/// ASCII-compatible (UTF-16: a byte-order mark or a zero byte), whose bytes
/// cannot tell what it contains.
fn not_ascii_compatible(head: &[u8]) -> bool {
    let head = &head[..head.len().min(4)];
    head.starts_with(&[0xFE, 0xFF]) || head.starts_with(&[0xFF, 0xFE]) || head.contains(&0)
}

/// For each needle, whether the bytes of `path` may contain it: `false` only
/// when a document in an ASCII-compatible encoding certainly does not. A
/// document that may be UTF-16 is assumed to contain every needle. One read
/// of the file.
pub fn may_contain<const N: usize>(path: &Path, needles: [&[u8]; N]) -> Result<[bool; N]> {
    may_contain_in_chunks(path, needles, 8 << 20)
}

/// Whether the file at `path` may contain any of `needles`.
fn may_contain_any(path: &Path, needles: &[&[u8]]) -> Result<bool> {
    let io = |e| Error::io(format!("cannot read {}", path.display()), e);
    let mut file = File::open(path).map_err(io)?;
    let finders: Vec<_> = needles.iter().map(memchr::memmem::Finder::new).collect();
    let keep = needles.iter().map(|n| n.len()).max().unwrap_or(1).max(1) - 1;
    let mut buf = vec![0u8; (8 << 20).max(keep + 1)];
    let mut filled = 0;
    loop {
        let read = file.read(&mut buf[filled..]).map_err(io)?;
        let end = filled + read;
        if finders.iter().any(|f| f.find(&buf[..end]).is_some()) {
            return Ok(true);
        }
        if read == 0 {
            return Ok(false);
        }
        let tail = end.saturating_sub(keep);
        buf.copy_within(tail..end, 0);
        filled = end - tail;
    }
}

fn may_contain_in_chunks<const N: usize>(
    path: &Path,
    needles: [&[u8]; N],
    chunk: usize,
) -> Result<[bool; N]> {
    let io = |e| Error::io(format!("cannot read {}", path.display()), e);
    let mut file = File::open(path).map_err(io)?;
    let finders = needles.map(memchr::memmem::Finder::new);
    let keep = needles.iter().map(|n| n.len()).max().unwrap_or(1).max(1) - 1;
    let mut found = [false; N];
    let mut buf = vec![0u8; chunk.max(keep + 1)];
    let mut filled = 0;
    let mut first = true;
    loop {
        let read = file.read(&mut buf[filled..]).map_err(io)?;
        if std::mem::take(&mut first) && not_ascii_compatible(&buf[..read]) {
            return Ok([true; N]);
        }
        let end = filled + read;
        for (hit, finder) in found.iter_mut().zip(&finders) {
            *hit = *hit || finder.find(&buf[..end]).is_some();
        }
        if read == 0 || found.iter().all(|&hit| hit) {
            return Ok(found);
        }
        // Carry the tail over, so a needle across the boundary is found.
        let tail = end.saturating_sub(keep);
        buf.copy_within(tail..end, 0);
        filled = end - tail;
    }
}

/// Whether the document at `path` binds an ADE's namespace on any element
/// and spells an element in it, judged from its bytes: every `xmlns`
/// declaration whose URI is an ADE's ([`is_ade`]), then an element of a
/// prefix one binds (`<uro:`), or any element at all for an ADE bound as the
/// default namespace.
pub fn uses_ade(path: &Path) -> Result<bool> {
    // Enough to see a whole namespace declaration after `xmlns`.
    const LOOKAHEAD: usize = 4096;
    let io = |e| Error::io(format!("cannot read {}", path.display()), e);
    let mut file = File::open(path).map_err(io)?;
    let finder = memchr::memmem::Finder::new(b"xmlns");
    let mut prefixes: Vec<Vec<u8>> = Vec::new();
    let mut buf = vec![0u8; 8 << 20];
    let mut filled = 0;
    let mut first = true;
    loop {
        let read = file.read(&mut buf[filled..]).map_err(io)?;
        if std::mem::take(&mut first) && not_ascii_compatible(&buf[..read]) {
            return Ok(true);
        }
        let end = filled + read;
        let eof = read == 0;
        // A declaration starting in the last `LOOKAHEAD` bytes waits for the
        // next chunk, which starts with them.
        let limit = if eof {
            end
        } else {
            end.saturating_sub(LOOKAHEAD)
        };
        for at in finder.find_iter(&buf[..end]).take_while(|&at| at < limit) {
            match namespace_declaration(&buf[at + 5..end]) {
                Some((prefix, uri)) if is_ade(uri) => {
                    if prefix.is_empty() {
                        return Ok(true);
                    }
                    if !prefixes.iter().any(|p| p == prefix) {
                        prefixes.push(prefix.to_vec());
                    }
                }
                _ => {}
            }
        }
        if eof {
            break;
        }
        buf.copy_within(limit..end, 0);
        filled = end - limit;
    }
    if prefixes.is_empty() {
        return Ok(false);
    }
    let needles: Vec<Vec<u8>> = prefixes
        .iter()
        .map(|p| [b"<".as_slice(), p, b":"].concat())
        .collect();
    let needles: Vec<&[u8]> = needles.iter().map(Vec::as_slice).collect();
    may_contain_any(path, &needles)
}

/// The prefix (empty for a default namespace) and URI of the namespace
/// declaration `rest` begins with, just after its `xmlns`.
fn namespace_declaration(rest: &[u8]) -> Option<(&[u8], &[u8])> {
    let (prefix, rest) = match rest.first()? {
        b':' => {
            let name = rest[1..]
                .iter()
                .position(|&b| b == b'=' || b.is_ascii_whitespace())?;
            (&rest[1..1 + name], &rest[1 + name..])
        }
        _ => (&rest[..0], rest),
    };
    let rest = rest
        .trim_ascii_start()
        .strip_prefix(b"=")?
        .trim_ascii_start();
    let quote = *rest.first()?;
    if quote != b'"' && quote != b'\'' {
        return None;
    }
    let len = rest[1..].iter().position(|&b| b == quote)?;
    Some((prefix, &rest[1..1 + len]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data(name: &str) -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../citygml2/tests/data")
            .join(name)
    }

    /// The byte search that lets the CityModel-appearance pass be skipped
    /// finds the element across the boundary between two reads, exercised
    /// with reads shorter than the needle, and reports a document without it.
    #[test]
    fn the_appearance_member_search_finds_it_across_read_boundaries() {
        let with = data("building_citymodel_appearance.gml");
        let without = data("b1_lod2_cs_w_sem.gml");
        for chunk in [5, 7, 4096, 8 << 20] {
            assert_eq!(
                may_contain_in_chunks(&with, [b"appearanceMember"], chunk).unwrap(),
                [true],
                "{chunk}"
            );
            assert_eq!(
                may_contain_in_chunks(&without, [b"appearanceMember"], chunk).unwrap(),
                [false],
                "{chunk}"
            );
        }
    }
}
