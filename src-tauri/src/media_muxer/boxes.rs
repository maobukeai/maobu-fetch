use std::{io::SeekFrom, path::Path};
use tokio::{
    fs::File,
    io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt},
};
use tokio_util::sync::CancellationToken;

pub const MAX_METADATA: u64 = 8 * 1024 * 1024;
pub const MAX_FRAGMENT: u64 = 4 * 1024 * 1024;
pub const COPY_BUFFER: usize = 256 * 1024;
#[derive(Clone, Copy, Debug)]
pub struct BoxHeader {
    pub offset: u64,
    pub size: u64,
    pub header: usize,
    pub kind: [u8; 4],
    pub zero: bool,
}
#[derive(Clone, Copy, Debug)]
pub struct Child {
    pub offset: usize,
    pub size: usize,
    pub header: usize,
    pub kind: [u8; 4],
}
pub fn running(cancel: &CancellationToken) -> Result<(), String> {
    if cancel.is_cancelled() {
        Err("媒体合并已取消，原始轨道已保留".into())
    } else {
        Ok(())
    }
}
pub fn u32_at(data: &[u8], offset: usize) -> Result<u32, String> {
    let bytes = data
        .get(offset..offset.checked_add(4).ok_or("MP4 字段偏移溢出")?)
        .ok_or("MP4 字段超出 box 范围")?;
    Ok(u32::from_be_bytes(
        bytes.try_into().map_err(|_| "MP4 字段无效")?,
    ))
}
pub fn u64_at(data: &[u8], offset: usize) -> Result<u64, String> {
    let bytes = data
        .get(offset..offset.checked_add(8).ok_or("MP4 字段偏移溢出")?)
        .ok_or("MP4 字段超出 box 范围")?;
    Ok(u64::from_be_bytes(
        bytes.try_into().map_err(|_| "MP4 字段无效")?,
    ))
}
pub fn put_u32(data: &mut [u8], offset: usize, value: u32) -> Result<(), String> {
    data.get_mut(offset..offset.checked_add(4).ok_or("MP4 字段偏移溢出")?)
        .ok_or("MP4 字段超出 box 范围")?
        .copy_from_slice(&value.to_be_bytes());
    Ok(())
}
pub fn put_u64(data: &mut [u8], offset: usize, value: u64) -> Result<(), String> {
    data.get_mut(offset..offset.checked_add(8).ok_or("MP4 字段偏移溢出")?)
        .ok_or("MP4 字段超出 box 范围")?
        .copy_from_slice(&value.to_be_bytes());
    Ok(())
}
pub fn children(data: &[u8]) -> Result<Vec<Child>, String> {
    let mut result = Vec::new();
    let mut offset = 0;
    while offset < data.len() {
        if result.len() >= 8192 {
            return Err("MP4 子 box 数量超过安全上限".into());
        }
        let size = u32_at(data, offset)?;
        let kind: [u8; 4] = data
            .get(offset + 4..offset + 8)
            .ok_or("MP4 box 头不完整")?
            .try_into()
            .map_err(|_| "MP4 box 类型无效")?;
        let header = if size == 1 { 16 } else { 8 };
        let size = match size {
            0 => return Err("MP4 嵌套 box 不得使用未定长度".into()),
            1 => usize::try_from(u64_at(data, offset + 8)?).map_err(|_| "MP4 box 过大")?,
            n => n as usize,
        };
        if size < header || offset.checked_add(size).is_none_or(|end| end > data.len()) {
            return Err("MP4 box 长度无效或截断".into());
        }
        result.push(Child {
            offset,
            size,
            header,
            kind,
        });
        offset += size;
    }
    Ok(result)
}
pub fn only(data: &[u8], kind: &[u8; 4]) -> Result<Child, String> {
    let found: Vec<_> = children(data)?
        .into_iter()
        .filter(|item| &item.kind == kind)
        .collect();
    if found.len() != 1 {
        return Err(format!(
            "MP4 必须恰好包含一个 {} box",
            String::from_utf8_lossy(kind)
        ));
    }
    Ok(found[0])
}
pub fn payload(data: &[u8], child: Child) -> &[u8] {
    &data[child.offset + child.header..child.offset + child.size]
}
pub fn boxed(kind: &[u8; 4], data: &[u8]) -> Result<Vec<u8>, String> {
    let length = data.len().checked_add(8).ok_or("MP4 box 长度溢出")?;
    if length as u64 > 2 * MAX_METADATA {
        return Err("MP4 元数据超过安全上限".into());
    }
    let mut out = Vec::new();
    out.try_reserve_exact(length)
        .map_err(|_| "MP4 元数据内存不足")?;
    out.extend_from_slice(&(length as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    Ok(out)
}
pub async fn read_header(
    file: &mut File,
    offset: u64,
    length: u64,
    cancel: &CancellationToken,
) -> Result<Option<BoxHeader>, String> {
    running(cancel)?;
    if offset == length {
        return Ok(None);
    };
    if length.saturating_sub(offset) < 8 {
        return Err("MP4 顶级 box 头截断".into());
    }
    file.seek(SeekFrom::Start(offset)).await.map_err(io_error)?;
    let mut bytes = [0u8; 16];
    file.read_exact(&mut bytes[..8]).await.map_err(io_error)?;
    let declared = u32_at(&bytes, 0)?;
    let header = if declared == 1 { 16 } else { 8 };
    if declared == 1 {
        file.read_exact(&mut bytes[8..]).await.map_err(io_error)?;
    }
    let size = match declared {
        0 => length - offset,
        1 => u64_at(&bytes, 8)?,
        n => u64::from(n),
    };
    if size < header as u64 || offset.checked_add(size).is_none_or(|end| end > length) {
        return Err("MP4 顶级 box 长度无效或超出文件".into());
    }
    Ok(Some(BoxHeader {
        offset,
        size,
        header,
        kind: bytes[4..8].try_into().map_err(|_| "MP4 box 类型无效")?,
        zero: declared == 0,
    }))
}
pub async fn read_small(
    file: &mut File,
    item: BoxHeader,
    limit: u64,
    cancel: &CancellationToken,
) -> Result<Vec<u8>, String> {
    running(cancel)?;
    if item.size > limit {
        return Err("MP4 元数据 box 超过有界内存上限，请使用受支持的媒体格式".into());
    }
    let size = usize::try_from(item.size).map_err(|_| "MP4 元数据长度无效")?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(size)
        .map_err(|_| "MP4 元数据内存不足")?;
    bytes.resize(size, 0);
    file.seek(SeekFrom::Start(item.offset))
        .await
        .map_err(io_error)?;
    for chunk in bytes.chunks_mut(COPY_BUFFER) {
        running(cancel)?;
        file.read_exact(chunk).await.map_err(io_error)?;
    }
    Ok(bytes)
}
pub async fn copy_box(
    file: &mut File,
    out: &mut File,
    item: BoxHeader,
    buffer: &mut [u8],
    cancel: &CancellationToken,
) -> Result<u64, String> {
    running(cancel)?;
    file.seek(SeekFrom::Start(item.offset))
        .await
        .map_err(io_error)?;
    let mut remaining = item.size;
    if item.zero {
        let size = u32::try_from(item.size)
            .map_err(|_| "超过 4 GiB 的未定长度 mdat 暂不支持，请使用明确长度的 fMP4")?;
        let mut header = [0; 8];
        file.read_exact(&mut header).await.map_err(io_error)?;
        header[..4].copy_from_slice(&size.to_be_bytes());
        out.write_all(&header).await.map_err(io_error)?;
        remaining -= 8;
    }
    while remaining > 0 {
        running(cancel)?;
        let length = remaining.min(buffer.len() as u64) as usize;
        file.read_exact(&mut buffer[..length])
            .await
            .map_err(io_error)?;
        out.write_all(&buffer[..length]).await.map_err(io_error)?;
        remaining -= length as u64;
    }
    Ok(item.size)
}
pub async fn ordinary(path: &Path) -> Result<(), String> {
    if !path.is_absolute() {
        return Err("媒体文件必须使用明确的绝对路径".into());
    }
    let meta = tokio::fs::symlink_metadata(path).await.map_err(io_error)?;
    if !meta.is_file() || meta.file_type().is_symlink() {
        return Err("媒体输入不是安全的普通文件".into());
    }
    Ok(())
}
pub fn io_error(error: std::io::Error) -> String {
    format!("媒体文件读写失败：{error}")
}
