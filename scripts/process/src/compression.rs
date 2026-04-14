use std::io::{Cursor, Read};

use zip::ZipArchive;

/// If the file is a zip file then attempt to unzip it. Fail if it contains more than one file.
/// If the file is not a zip file then simply return the raw file data.
pub fn maybe_unzip_single_file(raw_file: Vec<u8>) -> Vec<u8> {
    let cursor = Cursor::new(&raw_file);
    let Ok(mut archive) = ZipArchive::new(cursor) else {
        return raw_file;
    };

    assert!(archive.file_names().count() == 1);

    let mut file = archive.by_index(0).unwrap();
    let mut out = Vec::new();
    file.read_to_end(&mut out).unwrap();

    out
}

pub fn zstd_decode(file: &[u8]) -> Vec<u8> {
  zstd::decode_all(Cursor::new(&file)).unwrap()
}

pub fn zstd_encode(file: &[u8], level: u16) -> Vec<u8> {
  zstd::encode_all(Cursor::new(&file), level as i32).unwrap()
}
