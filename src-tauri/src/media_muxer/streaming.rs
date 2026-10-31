use super::{
    boxes::*,
    fragments,
    metadata::{self, Input},
};
use std::{
    io::SeekFrom,
    path::{Path, PathBuf},
};
use tokio::{
    fs::{self, File, OpenOptions},
    io::{AsyncSeekExt, AsyncWriteExt},
};
use tokio_util::sync::CancellationToken;

pub async fn merge(
    video: &Path,
    audio: &Path,
    output: &Path,
    cancel: &CancellationToken,
) -> Result<(), String> {
    running(cancel)?;
    if !output.is_absolute() {
        return Err("媒体输出必须使用明确的绝对路径".into());
    }
    let parent = output.parent().ok_or("媒体输出缺少明确目录")?;
    let output_name = output.file_name().ok_or("媒体输出文件名无效")?;
    let canonical_parent = fs::canonicalize(parent).await.map_err(io_error)?;
    let output = canonical_parent.join(output_name);
    for source in [video, audio] {
        if fs::canonicalize(source).await.map_err(io_error)? == output {
            return Err("合并输出不能覆盖原始输入轨道".into());
        }
    }
    if fs::try_exists(&output).await.map_err(io_error)? {
        return Err("媒体合并目标已存在，已保留原文件".into());
    }
    let mut video = Input::open(video, cancel).await?;
    let mut audio = Input::open(audio, cancel).await?;
    let moov = metadata::merged(&video.metadata, &audio.metadata)?;
    let temporary = PathBuf::from(format!(
        "{}.merging-{}",
        output.to_string_lossy(),
        uuid::Uuid::new_v4()
    ));
    let mut out = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)
        .await
        .map_err(io_error)?;
    let mut buffer = vec![0; COPY_BUFFER];
    let mut written = 0;
    let mut sequence = 0;
    // Preserve only initialization-prefix boxes; moov itself is rebuilt once.
    let mut offset = 0;
    while offset < video.moov_offset {
        let item = read_header(&mut video.file, offset, video.length, cancel)
            .await?
            .ok_or("MP4 初始化段截断")?;
        if !matches!(&item.kind, b"ftyp" | b"free" | b"skip") {
            return Err("moov 前存在不支持的关联元数据".into());
        }
        written += copy_box(&mut video.file, &mut out, item, &mut buffer, cancel).await?;
        offset += item.size;
    }
    out.write_all(&moov).await.map_err(io_error)?;
    written += moov.len() as u64;
    for (input, id) in [(&mut video, 1), (&mut audio, 2)] {
        write_fragments(
            input,
            &mut out,
            &mut written,
            &mut sequence,
            id,
            &mut buffer,
            cancel,
        )
        .await?;
    }
    running(cancel)?;
    out.flush().await.map_err(io_error)?;
    out.sync_all().await.map_err(io_error)?;
    drop(out);
    if fs::metadata(&temporary).await.map_err(io_error)?.len() != written {
        return Err("媒体合并输出长度校验失败，原始轨道已保留".into());
    }
    running(cancel)?;
    crate::manager::publish_file_atomically(&temporary, &output, false).await?;
    Ok(())
}

async fn write_fragments(
    input: &mut Input,
    out: &mut File,
    written: &mut u64,
    sequence: &mut u32,
    new_id: u32,
    buffer: &mut [u8],
    cancel: &CancellationToken,
) -> Result<(), String> {
    let mut offset = input.fragments;
    let mut count = 0;
    while let Some(item) = read_header(&mut input.file, offset, input.length, cancel).await? {
        match &item.kind {
            b"moof" => {
                let mut bytes = read_small(&mut input.file, item, MAX_FRAGMENT, cancel).await?;
                let mut mdats = Vec::new();
                let mut next = offset + item.size;
                while let Some(media) =
                    read_header(&mut input.file, next, input.length, cancel).await?
                {
                    if media.kind != *b"mdat" {
                        break;
                    }
                    if mdats.len() >= 1024 {
                        return Err("单个片段的 mdat 数量超过安全上限".into());
                    }
                    if media.size <= media.header as u64 {
                        return Err("mdat 媒体数据为空".into());
                    }
                    mdats.push(media);
                    next += media.size;
                }
                if mdats.is_empty() {
                    return Err("moof 后缺少完整 mdat 媒体数据".into());
                }
                *sequence = sequence.checked_add(1).ok_or("MP4 片段序列溢出")?;
                fragments::patch(
                    &mut bytes,
                    item.header,
                    item.offset,
                    *written,
                    &mdats,
                    &input.metadata,
                    new_id,
                    *sequence,
                )?;
                running(cancel)?;
                out.write_all(&bytes).await.map_err(io_error)?;
                *written += bytes.len() as u64;
                for media in mdats {
                    *written += copy_box(&mut input.file, out, media, buffer, cancel).await?;
                }
                offset = next;
                count += 1;
                continue;
            }
            // These indexes hold original absolute offsets; omitting them is
            // valid for an unindexed fragmented file, copying them would corrupt seeks.
            b"sidx" | b"mfra" => {}
            b"styp" | b"emsg" | b"free" | b"skip" => {
                *written += copy_box(&mut input.file, out, item, buffer, cancel).await?;
            }
            b"prft" => {
                let mut bytes = read_small(&mut input.file, item, 4096, cancel).await?;
                put_u32(&mut bytes, item.header + 4, new_id)?;
                out.write_all(&bytes).await.map_err(io_error)?;
                *written += bytes.len() as u64;
            }
            _ => return Err("媒体片段布局无效，拒绝写出不完整 MP4".into()),
        }
        offset += item.size;
    }
    if count == 0 {
        return Err("输入轨道没有有效媒体片段".into());
    }
    input
        .file
        .seek(SeekFrom::Start(input.length))
        .await
        .map_err(io_error)?;
    Ok(())
}
