use std::path::Path;

use rayon::prelude::*;
use rpf_archive::crypto::GtaKeys;
use sha1::{Digest, Sha1};

use crate::error::{Result, SqueezeError};

static NG_DATA: &[u8] = include_bytes!("ng_keys.dat");

const AES_KEY_LEN: usize = 32;
const NG_KEY_LEN: usize = 272;
const NG_KEY_COUNT: usize = 101;

const PC_AES_KEY_HASH: [u8; 20] = [
    0xA0, 0x79, 0x61, 0x28, 0xA7, 0x75, 0x72, 0x0A, 0xC2, 0x04, 0xD9, 0x81, 0x9F, 0x68, 0xC1, 0x72,
    0xE3, 0x95, 0x2C, 0x6D,
];

pub fn from_exe(exe: &Path) -> Result<GtaKeys> {
    let bytes = std::fs::read(exe).map_err(|e| SqueezeError::io(exe, e))?;
    tracing::info!(
        mib = bytes.len() / (1024 * 1024),
        "scanning GTA5.exe for keys"
    );

    let aes_key = find_aes_key(&bytes).ok_or_else(|| SqueezeError::Keys {
        path: exe.to_path_buf(),
        detail: "no AES key blob in this executable — wrong file or a patched build".into(),
    })?;
    tracing::info!("AES key found");

    Ok(build_keys(aes_key))
}

fn find_aes_key(data: &[u8]) -> Option<[u8; AES_KEY_LEN]> {
    let last = data.len().checked_sub(AES_KEY_LEN)?;
    (0..=last).into_par_iter().find_map_first(|i| {
        let window = &data[i..i + AES_KEY_LEN];
        (Sha1::digest(window).as_slice() == PC_AES_KEY_HASH).then(|| {
            let mut key = [0u8; AES_KEY_LEN];
            key.copy_from_slice(window);
            key
        })
    })
}

fn build_keys(aes_key: [u8; AES_KEY_LEN]) -> GtaKeys {
    let (ng_blob, table_blob) = NG_DATA.split_at(NG_KEY_COUNT * NG_KEY_LEN);

    let ng_keys = ng_blob
        .chunks_exact(NG_KEY_LEN)
        .map(<[u8]>::to_vec)
        .collect();

    let mut tables = Box::new([[[0u32; 256]; 16]; 17]);
    let mut words = table_blob.chunks_exact(4);
    for table in tables.iter_mut().flatten().flatten() {
        let Some(word) = words.next() else { break };
        *table = u32::from_le_bytes(word.try_into().expect("chunks_exact(4)"));
    }

    GtaKeys {
        aes_key,
        ng_keys,
        ng_decrypt_tables: tables,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_key_blob_is_the_size_we_expect() {
        let tables = 17 * 16 * 256 * 4;
        assert_eq!(NG_DATA.len(), NG_KEY_COUNT * NG_KEY_LEN + tables);
    }

    #[test]
    fn tables_unpack_fully() {
        let keys = build_keys([7u8; AES_KEY_LEN]);
        assert_eq!(keys.ng_keys.len(), NG_KEY_COUNT);
        assert!(keys.ng_keys.iter().all(|k| k.len() == NG_KEY_LEN));
    }
}
