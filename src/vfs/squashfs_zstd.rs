//! SquashFS reading with Brum's own zstd (#150).
//!
//! `backhand` 0.25 still needs an older zstd with known security advisories. Brum turns off
//! backhand's built-in zstd and plugs in this small decompressor instead, which uses the same
//! up-to-date zstd as the rest of Brum. Every other SquashFS compression (gzip, xz, lz4) is
//! handled by backhand as before.
//!
//! When backhand moves to zstd-safe 8, this file can go and backhand's `zstd` feature can be
//! turned back on.

use std::io::{BufRead, Read, Seek};

use backhand::compression::{CompressionAction, Compressor, DefaultCompressor};
use backhand::kind::Kind;
use backhand::{BackhandError, FilesystemCompressor, FilesystemReader, SuperBlock};

/// backhand's own decompressor, with zstd blocks handed to Brum's zstd
pub struct BrumSquashfsCompressor;

pub static BRUM_SQUASHFS_COMPRESSOR: BrumSquashfsCompressor = BrumSquashfsCompressor;

impl CompressionAction for BrumSquashfsCompressor {
    type Error = BackhandError;
    type Compressor = Compressor;
    type FilesystemCompressor = FilesystemCompressor;
    type SuperBlock = SuperBlock;

    fn decompress(&self, bytes: &[u8], out: &mut Vec<u8>, compressor: Compressor) -> Result<(), BackhandError> {
        match compressor {
            Compressor::Zstd => {
                // Same call backhand makes: decompress into the space already reserved in `out`
                let mut decoder = zstd::bulk::Decompressor::new()?;
                decoder.decompress_to_buffer(bytes, out)?;
                Ok(())
            }
            other => DefaultCompressor.decompress(bytes, out, other),
        }
    }

    fn compress(&self, bytes: &[u8], fc: FilesystemCompressor, block_size: u32) -> Result<Vec<u8>, BackhandError> {
        // Brum only reads SquashFS images. Writing zstd is here so tests can build zstd images:
        // with its zstd turned off, backhand reports zstd as unsupported, and Brum's zstd steps in.
        match DefaultCompressor.compress(bytes, fc, block_size) {
            Err(BackhandError::UnsupportedCompression(_)) => Ok(zstd::bulk::compress(bytes, 3)?),
            other => other,
        }
    }
}

/// Opens a SquashFS image (little-endian v4, the standard kind) with Brum's zstd
pub fn open_squashfs<'b, R>(reader: R) -> Result<FilesystemReader<'b>, BackhandError>
where
    R: BufRead + Read + Seek + Send + 'b,
{
    FilesystemReader::from_reader_with_offset_and_kind(reader, 0, Kind::new_v4(&BRUM_SQUASHFS_COMPRESSOR))
}

#[cfg(test)]
mod tests {
    use super::*;
    use backhand::{FilesystemWriter, NodeHeader};
    use std::io::Cursor;

    fn build_image(compressor: Compressor) -> Vec<u8> {
        let header = NodeHeader { permissions: 0o644, uid: 1000, gid: 1000, mtime: 1_700_000_000 };
        let dir = NodeHeader { permissions: 0o755, ..header };
        let mut fs = FilesystemWriter::default();
        fs.set_kind(Kind::new_v4(&BRUM_SQUASHFS_COMPRESSOR));
        fs.set_compressor(FilesystemCompressor::new(compressor, None).unwrap());
        // Big enough to span several data blocks, so real compressed blocks are read back
        let big: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
        fs.push_file(Cursor::new(b"Hello zstd SquashFS".to_vec()), "hello.txt", header).unwrap();
        fs.push_dir("sub", dir).unwrap();
        fs.push_file(Cursor::new(big), "sub/big.bin", header).unwrap();
        let mut out = Cursor::new(Vec::new());
        fs.write(&mut out).unwrap();
        out.into_inner()
    }

    fn read_back(image: Vec<u8>) {
        let reader = open_squashfs(std::io::BufReader::new(Cursor::new(image))).unwrap();
        let mut found = 0;
        for node in reader.files() {
            if let backhand::InnerNode::File(f) = &node.inner {
                let mut data = Vec::new();
                reader.file(f).reader().read_to_end(&mut data).unwrap();
                match node.fullpath.to_str().unwrap() {
                    "/hello.txt" => { assert_eq!(data, b"Hello zstd SquashFS"); found += 1; }
                    "/sub/big.bin" => {
                        assert_eq!(data.len(), 300_000);
                        assert!(data.iter().enumerate().all(|(i, b)| *b == (i as u32 % 251) as u8));
                        found += 1;
                    }
                    other => panic!("unexpected file {other}"),
                }
            }
        }
        assert_eq!(found, 2);
    }

    #[test]
    fn reads_zstd_squashfs() {
        read_back(build_image(Compressor::Zstd));
    }

    #[test]
    fn other_compressions_still_use_backhand() {
        for c in [Compressor::Gzip, Compressor::Xz] {
            read_back(build_image(c));
        }
    }
}
