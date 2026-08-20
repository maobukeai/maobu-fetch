// HLS AES-128-CBC 原生解密器（纯 Rust，基于 RustCrypto aes/cbc）

use aes::Aes128;
use cbc::Decryptor;
use cbc::cipher::{BlockDecryptMut, KeyIvInit, block_padding::{NoPadding, Pkcs7}};

type Aes128CbcDec = Decryptor<Aes128>;

/// 解密 AES-128-CBC 密文
pub fn decrypt_aes_128(key: &[u8], iv: &[u8; 16], ciphertext: &[u8]) -> Result<Vec<u8>, String> {
    if key.len() != 16 {
        return Err(format!(
            "AES-128 密钥长度必须为 16 字节，实际为 {} 字节",
            key.len()
        ));
    }
    if ciphertext.is_empty() {
        return Ok(Vec::new());
    }
    if ciphertext.len() % 16 != 0 {
        return Err(format!(
            "AES-128 密文长度必须为 16 字节的倍数，实际为 {} 字节",
            ciphertext.len()
        ));
    }

    let mut buf = ciphertext.to_vec();
    let decryptor = Aes128CbcDec::new(key.into(), iv.into());

    // 优先尝试标准 PKCS#7 填充解密
    match decryptor.decrypt_padded_mut::<Pkcs7>(&mut buf) {
        Ok(decrypted) => Ok(decrypted.to_vec()),
        Err(_) => {
            // 部分 HLS TS 流使用整块传输（末尾无标准 PKCS#7 填充），回退至 NoPadding
            let raw_dec = Aes128CbcDec::new(key.into(), iv.into());
            let mut raw_buf = ciphertext.to_vec();
            match raw_dec.decrypt_padded_mut::<NoPadding>(&mut raw_buf) {
                Ok(raw_res) => Ok(raw_res.to_vec()),
                Err(e) => Err(format!("AES-128-CBC 解密失败: {e:?}")),
            }
        }
    }
}

/// 根据 RFC 8216 §5.2 规范：若 #EXT-X-KEY 未显式指定 IV 属性，
/// 则将媒体切片的序列号（Media Sequence Number）作为 128 位大端无符号整数作为 IV。
pub fn derive_iv_from_sequence(sequence: u64) -> [u8; 16] {
    let mut iv = [0u8; 16];
    let seq_bytes = sequence.to_be_bytes();
    iv[8..16].copy_from_slice(&seq_bytes);
    iv
}
