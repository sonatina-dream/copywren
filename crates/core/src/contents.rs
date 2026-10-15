//! Compact binary encoding of `ClipContents` for the `item_data` table.
//!
//! Layout (all integers little-endian):
//! `b"CWC1"`, `u32` entry count, then per entry a `u32` representation count,
//! then per representation `u32` type length, type bytes, `u64` data length,
//! data bytes.

use anyhow::{Context, Result, bail};

use crate::model::{ClipContents, PasteboardEntry, Representation};

const MAGIC: &[u8; 4] = b"CWC1";

pub fn encode(contents: &ClipContents) -> Vec<u8> {
    let mut out = Vec::with_capacity(contents.total_bytes() + 64);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&(contents.entries.len() as u32).to_le_bytes());
    for entry in &contents.entries {
        out.extend_from_slice(&(entry.representations.len() as u32).to_le_bytes());
        for rep in &entry.representations {
            out.extend_from_slice(&(rep.uti.len() as u32).to_le_bytes());
            out.extend_from_slice(rep.uti.as_bytes());
            out.extend_from_slice(&(rep.data.len() as u64).to_le_bytes());
            out.extend_from_slice(&rep.data);
        }
    }
    out
}

pub fn decode(bytes: &[u8]) -> Result<ClipContents> {
    let mut reader = Reader { bytes, pos: 0 };
    if reader.take(4)? != MAGIC {
        bail!("not a Copywren clip payload");
    }
    let entry_count = reader.u32()? as usize;
    let mut entries = Vec::with_capacity(entry_count.min(1024));
    for _ in 0..entry_count {
        let rep_count = reader.u32()? as usize;
        let mut representations = Vec::with_capacity(rep_count.min(1024));
        for _ in 0..rep_count {
            let uti_len = reader.u32()? as usize;
            let uti = std::str::from_utf8(reader.take(uti_len)?)
                .context("type identifier is not UTF-8")?
                .to_string();
            let data_len = usize::try_from(reader.u64()?).context("payload too large")?;
            let data = reader.take(data_len)?.to_vec();
            representations.push(Representation { uti, data });
        }
        entries.push(PasteboardEntry { representations });
    }
    if reader.pos != bytes.len() {
        bail!("trailing bytes after clip payload");
    }
    Ok(ClipContents { entries })
}

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, len: usize) -> Result<&'a [u8]> {
        let end = self.pos.checked_add(len).context("length overflow")?;
        let slice = self
            .bytes
            .get(self.pos..end)
            .context("truncated clip payload")?;
        self.pos = end;
        Ok(slice)
    }

    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into()?))
    }

    fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into()?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> ClipContents {
        ClipContents::new(vec![
            PasteboardEntry::new(vec![
                Representation::new("public.utf8-plain-text", "héllo"),
                Representation::new("public.rtf", vec![0u8, 1, 2, 255]),
            ]),
            PasteboardEntry::new(vec![Representation::new("public.png", Vec::<u8>::new())]),
            PasteboardEntry::default(),
        ])
    }

    #[test]
    fn round_trip() {
        let contents = sample();
        assert_eq!(decode(&encode(&contents)).unwrap(), contents);
        let empty = ClipContents::default();
        assert_eq!(decode(&encode(&empty)).unwrap(), empty);
    }

    #[test]
    fn rejects_corrupt_input() {
        let bytes = encode(&sample());
        assert!(decode(&bytes[..bytes.len() - 1]).is_err());
        assert!(decode(b"XXXX\0\0\0\0").is_err());
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert!(decode(&trailing).is_err());
        // An absurd length must fail cleanly rather than allocate.
        let mut huge = MAGIC.to_vec();
        huge.extend_from_slice(&1u32.to_le_bytes());
        huge.extend_from_slice(&1u32.to_le_bytes());
        huge.extend_from_slice(&u32::MAX.to_le_bytes());
        assert!(decode(&huge).is_err());
    }
}
