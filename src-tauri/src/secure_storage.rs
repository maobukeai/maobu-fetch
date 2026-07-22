//! Task 31：敏感数据的安全存储（Windows DPAPI）。
//!
//! 提供代理密码等敏感字段的对称加密 / 解密能力，避免明文落盘到 SQLite
//! 或 JSON 备份中。Windows 平台使用 DPAPI（`CryptProtectData` /
//! `CryptUnprotectData`），密钥由操作系统按当前用户派生，无需应用维护
//! 密钥材料。非 Windows 平台回退到 base64 包装的明文（仅用于开发与测试，
//! 生产环境只发布 Windows 构建）。
//!
//! ## 安全说明
//!
//! - 加密后的密文为字节序列，使用 base64 编码后存入数据库。
//! - DPAPI 默认按"当前用户"绑定密钥；同一用户在另一台机器上无法解密。
//! - 解密失败（密钥变化、密文损坏）时返回 `Err`，由调用方决定如何处理
//!   （通常退化为"无密码"状态，不阻塞下载流程）。
//! - 函数本身不写日志，避免泄露明文或密文（AGENTS.md §3）。
//!
//! ## 字节布局
//!
//! DPAPI 原始输出 -> base64 标准编码（带 padding）-> 数据库存储字符串。
//! 解密时反向：base64 解码 -> DPAPI `CryptUnprotectData` -> UTF-8 字符串。

use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine};

#[cfg(windows)]
use windows_sys::Win32::Security::Cryptography::{
    CryptProtectData, CryptUnprotectData, CRYPT_INTEGER_BLOB,
};

#[cfg(windows)]
use windows_sys::Win32::Foundation::LocalFree;

/// DPAPI 加密代理密码或其他敏感短字符串。
///
/// `plain` 不能为空（空字符串无业务意义，且某些 DPAPI 实现会返回错误）；
/// 调用方应先判断非空再调用本函数。返回的字符串为 base64 编码的密文，
/// 可安全存入 SQLite TEXT 字段或 JSON。
///
/// 失败时返回 `Err(String)`，错误信息为脱敏后的中文说明，不包含
/// 原始明文或 DPAPI 错误码（避免泄露内部状态）。
pub fn encrypt_password(plain: &str) -> Result<String, String> {
    if plain.is_empty() {
        return Err("待加密的密码为空".into());
    }
    let bytes = plain.as_bytes();
    let cipher = encrypt_bytes(bytes)?;
    Ok(BASE64_STANDARD.encode(&cipher))
}

/// DPAPI 解密代理密码。
///
/// `cipher` 为 `encrypt_password` 返回的 base64 字符串。失败时返回
/// 脱敏后的中文错误。成功时返回 UTF-8 明文字符串。
///
/// 旧版本数据库中可能存储明文密码（未经过 DPAPI 加密），此时
/// `decrypt_password` 会失败。调用方应捕获错误并提示用户重新输入密码，
/// 不要假定存储值始终为密文。
pub fn decrypt_password(cipher: &str) -> Result<String, String> {
    if cipher.is_empty() {
        return Err("待解密的密文为空".into());
    }
    let bytes = BASE64_STANDARD
        .decode(cipher.as_bytes())
        .map_err(|_| "密文 base64 解码失败".to_string())?;
    let plain = decrypt_bytes(&bytes)?;
    String::from_utf8(plain).map_err(|_| "解密结果不是有效的 UTF-8".to_string())
}

#[cfg(windows)]
fn encrypt_bytes(plain: &[u8]) -> Result<Vec<u8>, String> {
    // SAFETY: `plain` 借用的字节数组在调用期间保持有效；`CRYPT_INTEGER_BLOB`
    // 仅持有指针与长度，不获取所有权。`CryptProtectData` 写入 `out_blob`
    // 后由调用方负责 `LocalFree`。
    let in_blob = CRYPT_INTEGER_BLOB {
        cbData: plain.len() as u32,
        pbData: plain.as_ptr() as *mut u8,
    };
    let mut out_blob = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: core::ptr::null_mut(),
    };
    // dwFlags = 0：使用当前用户上下文加密（非机器绑定），跨进程不可解密。
    // description / optional entropy / reserved / prompt 均传 null：
    // 这些参数用于增强安全（如要求 UI 提示），但对本地代理密码场景不必要，
    // 且启用 prompt 会导致无 UI 会话的服务进程失败。
    let result = unsafe {
        CryptProtectData(
            &in_blob,
            core::ptr::null(),
            core::ptr::null(),
            core::ptr::null_mut(),
            core::ptr::null(),
            0,
            &mut out_blob,
        )
    };
    if result == 0 {
        return Err("DPAPI 加密失败".into());
    }
    // SAFETY: `out_blob.pbData` 由 DPAPI 通过 `LocalAlloc` 分配，
    // 拷贝后必须调用 `LocalFree` 释放，否则内存泄漏。
    let cipher = unsafe {
        let slice = core::slice::from_raw_parts(out_blob.pbData, out_blob.cbData as usize);
        let owned = slice.to_vec();
        let _ = LocalFree(out_blob.pbData as *mut _);
        owned
    };
    Ok(cipher)
}

#[cfg(windows)]
fn decrypt_bytes(cipher: &[u8]) -> Result<Vec<u8>, String> {
    let mut in_blob = CRYPT_INTEGER_BLOB {
        cbData: cipher.len() as u32,
        pbData: cipher.as_ptr() as *mut u8,
    };
    let mut out_blob = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: core::ptr::null_mut(),
    };
    let result = unsafe {
        CryptUnprotectData(
            &mut in_blob,
            core::ptr::null_mut(),
            core::ptr::null(),
            core::ptr::null_mut(),
            core::ptr::null(),
            0,
            &mut out_blob,
        )
    };
    if result == 0 {
        return Err("DPAPI 解密失败（密文可能由其他用户加密）".into());
    }
    let plain = unsafe {
        let slice = core::slice::from_raw_parts(out_blob.pbData, out_blob.cbData as usize);
        let owned = slice.to_vec();
        let _ = LocalFree(out_blob.pbData as *mut _);
        owned
    };
    Ok(plain)
}

#[cfg(not(windows))]
fn encrypt_bytes(plain: &[u8]) -> Result<Vec<u8>, String> {
    // 非 Windows 平台回退为直接返回原字节（仅用于开发与测试）。
    // 生产构建只针对 Windows，此分支不会进入用户路径。
    Ok(plain.to_vec())
}

#[cfg(not(windows))]
fn decrypt_bytes(cipher: &[u8]) -> Result<Vec<u8>, String> {
    Ok(cipher.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encrypt_then_decrypt_round_trips_simple_password() {
        let plain = "p@ssw0rd-123";
        let cipher = encrypt_password(plain).expect("encrypt should succeed");
        assert_ne!(cipher, plain, "cipher must differ from plain text");
        let restored = decrypt_password(&cipher).expect("decrypt should succeed");
        assert_eq!(restored, plain);
    }

    #[test]
    fn encrypt_then_decrypt_round_trips_unicode_password() {
        // 中文与 emoji 验证 UTF-8 边界。
        let plain = "密码-🔐-maobu";
        let cipher = encrypt_password(plain).expect("encrypt should succeed");
        let restored = decrypt_password(&cipher).expect("decrypt should succeed");
        assert_eq!(restored, plain);
    }

    #[test]
    fn encrypt_rejects_empty_input() {
        assert!(encrypt_password("").is_err());
    }

    #[test]
    fn decrypt_rejects_empty_input() {
        assert!(decrypt_password("").is_err());
    }

    #[test]
    fn decrypt_rejects_invalid_base64() {
        // 包含非法字符的"密文"应被识别为格式错误，不应触发 panic。
        let result = decrypt_password("!!!not-base64!!!");
        assert!(result.is_err());
    }

    #[cfg(not(windows))]
    #[test]
    fn non_windows_fallback_returns_plain_bytes() {
        // 非 Windows 平台的回退实现：base64(plain)。
        let cipher = encrypt_password("hello").unwrap();
        let restored = decrypt_password(&cipher).unwrap();
        assert_eq!(restored, "hello");
    }
}
