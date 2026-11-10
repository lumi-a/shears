//! Unpacks an uploaded `.zip` or `.tar.zst` archive into [`crate::ingest::RawFile`]s.

use std::io::{Cursor, Error as IoError, Read as _};
use std::path::Path;

use ruzstd::decoding::StreamingDecoder;
use ruzstd::decoding::errors::FrameDecoderError;
use zip::result::ZipError;

use crate::ingest::RawFile;

/// `.zip`'s local-file-header signature.
const ZIP_MAGIC: [u8; 4] = [b'P', b'K', 0x03, 0x04];
/// A zstd frame's magic number, the first four bytes of every `.tar.zst` file.
const ZSTD_MAGIC: [u8; 4] = [0x28, 0xB5, 0x2F, 0xFD];

/// A problem unpacking an uploaded archive.
#[derive(Debug, thiserror::Error)]
pub enum UnpackError {
    /// `bytes` didn't start with a recognized zip or zstd magic number.
    #[error("not a recognized archive: expected a .zip or .tar.zst file")]
    UnrecognizedFormat,
    /// The zstd stream itself was malformed.
    #[error("invalid zstd stream: {0}")]
    Zstd(#[from] FrameDecoderError),
    /// Reading from the zstd stream, or from the tar archive it decompresses to, failed.
    #[error("reading the archive: {0}")]
    Io(#[from] IoError),
    /// The zip archive itself was malformed.
    #[error("invalid zip archive: {0}")]
    Zip(#[from] ZipError),
}

/// Unpacks `bytes` (a `.zip` or `.tar.zst` file) into every file it contains.
///
/// # Errors
///
/// If `bytes` isn't a recognized archive format or the archive itself is corrupt.
#[inline]
pub fn unpack(bytes: &[u8]) -> Result<Vec<RawFile>, UnpackError> {
    if bytes.starts_with(&ZSTD_MAGIC) {
        unpack_tar_zst(bytes)
    } else if bytes.starts_with(&ZIP_MAGIC) {
        unpack_zip(bytes)
    } else {
        Err(UnpackError::UnrecognizedFormat)
    }
}

/// Decompresses `bytes` as a zstd stream and then unpacks the resulting tar stream.
fn unpack_tar_zst(bytes: &[u8]) -> Result<Vec<RawFile>, UnpackError> {
    /// Joins `path`'s components with `/` regardless of platform.
    fn slashed_path(path: &Path) -> String {
        path.components()
            .map(|component| component.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/")
    }

    let mut decoder = StreamingDecoder::new(bytes)?;
    let mut decompressed = Vec::new();
    let _: usize = decoder.read_to_end(&mut decompressed)?;

    let mut archive = tar::Archive::new(decompressed.as_slice());
    let mut files = Vec::new();
    for raw_entry in archive.entries()? {
        let mut entry = raw_entry?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let path = slashed_path(&entry.path()?);
        let mut contents = Vec::new();
        let _: usize = entry.read_to_end(&mut contents)?;
        files.push(RawFile {
            path,
            bytes: contents,
        });
    }
    Ok(files)
}

/// Unpacks `bytes` as a zip archive.
fn unpack_zip(bytes: &[u8]) -> Result<Vec<RawFile>, UnpackError> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))?;
    let mut files = Vec::with_capacity(archive.len());
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        if entry.is_dir() {
            continue;
        }
        // TODO: Should we use `enclosed_name` instead?
        // https://docs.rs/zip/9.5.1/zip/read/struct.ZipFile.html#method.enclosed_name
        // Either way, this doesn't guarantee that we get slash-separated paths, right?
        let path = entry.name()?.into_owned();
        let mut contents = Vec::new();
        let _: usize = entry.read_to_end(&mut contents)?;
        files.push(RawFile {
            path,
            bytes: contents,
        });
    }
    Ok(files)
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Write as _, empty as empty_reader};

    use zip::CompressionMethod;
    use zip::write::SimpleFileOptions;

    use super::{UnpackError, unpack};

    /// Builds a `.tar.zst` archive in memory, the same way the committed fixtures were made.
    fn make_tar_zst(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut tar_bytes = Vec::new();
        let mut builder = tar::Builder::new(&mut tar_bytes);
        for (path, contents) in files {
            let mut header = tar::Header::new_gnu();
            header.set_path(path).expect("a valid tar path");
            header.set_size(u64::try_from(contents.len()).expect("a test-sized file"));
            header.set_cksum();
            builder
                .append(&header, *contents)
                .expect("appending to an in-memory tar builder");
        }
        builder.finish().expect("finishing an in-memory tar");
        drop(builder);
        zstd::encode_all(tar_bytes.as_slice(), 0).expect("compressing an in-memory tar")
    }

    /// Builds a `.zip` archive in memory.
    fn make_zip(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options =
            SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        for (path, contents) in files {
            writer
                .start_file(*path, options)
                .expect("starting a zip entry");
            writer
                .write_all(contents)
                .expect("writing a zip entry's contents");
        }
        writer
            .finish()
            .expect("finishing an in-memory zip")
            .into_inner()
    }

    #[test]
    fn unpacks_a_tar_zst_archive() {
        let bytes = make_tar_zst(&[
            ("recipes/minecraft/dirt.json", b"{}"),
            ("index.json", b"[]"),
        ]);

        let mut files = unpack(&bytes).expect("a valid .tar.zst archive");
        files.sort_unstable_by(|a, b| a.path.cmp(&b.path));

        assert_eq!(
            files
                .iter()
                .map(|file| file.path.as_str())
                .collect::<Vec<_>>(),
            vec!["index.json", "recipes/minecraft/dirt.json"]
        );
        assert_eq!(files.first().expect("two files were unpacked").bytes, b"[]");
    }

    #[test]
    fn unpacks_a_zip_archive() {
        let bytes = make_zip(&[
            ("recipes/minecraft/dirt.json", b"{}"),
            ("index.json", b"[]"),
        ]);

        let mut files = unpack(&bytes).expect("a valid .zip archive");
        files.sort_unstable_by(|a, b| a.path.cmp(&b.path));

        assert_eq!(
            files
                .iter()
                .map(|file| file.path.as_str())
                .collect::<Vec<_>>(),
            vec!["index.json", "recipes/minecraft/dirt.json"]
        );
        assert_eq!(files.first().expect("two files were unpacked").bytes, b"[]");
    }

    #[test]
    fn skips_tar_directory_entries() {
        let mut tar_bytes = Vec::new();
        let mut builder = tar::Builder::new(&mut tar_bytes);
        let mut header = tar::Header::new_gnu();
        header.set_path("recipes/").expect("a valid tar path");
        header.set_entry_type(tar::EntryType::Directory);
        header.set_size(0);
        header.set_cksum();
        builder
            .append(&header, empty_reader())
            .expect("appending a directory entry");
        builder.finish().expect("finishing an in-memory tar");
        drop(builder);
        let bytes =
            zstd::encode_all(tar_bytes.as_slice(), 0).expect("compressing an in-memory tar");

        let files = unpack(&bytes).expect("a valid .tar.zst archive");

        assert_eq!(
            files.len(),
            0,
            "the directory entry shouldn't become a RawFile"
        );
    }

    #[test]
    fn skips_zip_directory_entries() {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        writer
            .add_directory("recipes/", SimpleFileOptions::default())
            .expect("adding a directory entry");
        let bytes = writer
            .finish()
            .expect("finishing an in-memory zip")
            .into_inner();

        let files = unpack(&bytes).expect("a valid .zip archive");

        assert_eq!(
            files.len(),
            0,
            "the directory entry shouldn't become a RawFile"
        );
    }

    #[test]
    fn rejects_an_unrecognized_format() {
        let error = unpack(b"not an archive at all").expect_err("garbage input should fail");
        assert!(matches!(error, UnpackError::UnrecognizedFormat));
    }

    #[test]
    fn rejects_a_corrupt_zip() {
        let mut bytes = make_zip(&[("recipes/minecraft/dirt.json", b"{}")]);
        // Corrupt a byte well past the local file header so the zip magic number still
        // matches, but the archive itself doesn't parse.
        let last = bytes.len() - 1;
        let corrupt_byte = bytes
            .get_mut(last)
            .expect("the archive has at least one byte");
        *corrupt_byte ^= 0xFF;

        let result = unpack(&bytes);
        assert!(result.is_err(), "a corrupted zip should fail to unpack");
    }
}
