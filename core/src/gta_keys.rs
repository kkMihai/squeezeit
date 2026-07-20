#![allow(clippy::needless_range_loop)]

use std::path::Path;

use sha1::{Digest, Sha1};

static NG_DATA: &[u8] = include_bytes!("ng_keys.dat");

static PC_AES_KEY_HASH: [u8; 20] = [
    0xA0, 0x79, 0x61, 0x28, 0xA7, 0x75, 0x72, 0x0A, 0xC2, 0x04, 0xD9, 0x81, 0x9F, 0x68, 0xC1, 0x72,
    0xE3, 0x95, 0x2C, 0x6D,
];

pub struct ExtractedKeys {
    pub aes_key: [u8; 32],
}

impl ExtractedKeys {
    pub fn extract_from_exe(exe_path: &Path) -> anyhow::Result<Self> {
        let file_size = std::fs::metadata(exe_path)?.len();
        tracing::info!(
            "[Keys] Reading GTA5.exe ({} MB)...",
            file_size / 1024 / 1024
        );

        let exe_data = std::fs::read(exe_path)?;
        tracing::info!("[Keys] Searching for AES key...");

        let aes_bytes = search_hash(&exe_data, &PC_AES_KEY_HASH, 32)
            .ok_or_else(|| anyhow::anyhow!("AES key not found in GTA5.exe"))?;

        let aes_key: [u8; 32] = aes_bytes.try_into().unwrap();
        tracing::info!("[Keys] AES key found");

        Ok(Self { aes_key })
    }

    pub fn to_rpf_archive_keys(&self) -> rpf_archive::crypto::GtaKeys {
        build_gta_keys(self.aes_key)
    }
}

fn build_gta_keys(aes_key: [u8; 32]) -> rpf_archive::crypto::GtaKeys {
    let mut offset = 0;

    let mut ng_keys = Vec::with_capacity(101);
    for _ in 0..101 {
        let mut key = vec![0u8; 272];
        key.copy_from_slice(&NG_DATA[offset..offset + 272]);
        ng_keys.push(key);
        offset += 272;
    }

    let mut tables = [[[0u32; 256]; 16]; 17];
    for i in 0..17 {
        for j in 0..16 {
            for k in 0..256 {
                tables[i][j][k] = u32::from_le_bytes([
                    NG_DATA[offset],
                    NG_DATA[offset + 1],
                    NG_DATA[offset + 2],
                    NG_DATA[offset + 3],
                ]);
                offset += 4;
            }
        }
    }

    rpf_archive::crypto::GtaKeys {
        aes_key,
        ng_keys,
        ng_decrypt_tables: Box::new(tables),
    }
}

fn search_hash(data: &[u8], expected_sha1: &[u8; 20], length: usize) -> Option<Vec<u8>> {
    if data.len() < length {
        return None;
    }
    for i in 0..=(data.len() - length) {
        let candidate = &data[i..i + length];
        let mut hasher = Sha1::new();
        hasher.update(candidate);
        let hash: [u8; 20] = hasher.finalize().into();
        if &hash == expected_sha1 {
            return Some(candidate.to_vec());
        }
    }
    None
}
