//! Validate one clear track or a video/audio pair without rewriting input data.
use super::{
    boxes::*,
    fragments,
    metadata::{self, Metadata, TrackKind},
};
use std::{collections::HashSet, path::Path};
use tokio::fs::File;
use tokio_util::sync::CancellationToken;

pub async fn validate(path: &Path, cancel: &CancellationToken) -> Result<(), String> {
    ordinary(path).await?;
    running(cancel)?;
    let mut file = File::open(path).await.map_err(io_error)?;
    let length = file.metadata().await.map_err(io_error)?.len();
    let mut offset = 0;
    let mut moov = None;
    let mut first_fragment = None;
    let mut ftyp = false;
    let mut count = 0;
    while let Some(item) = read_header(&mut file, offset, length, cancel).await? {
        count += 1;
        if count > 1_000_000 {
            return Err("MP4 box 数量超过安全上限".into());
        }
        match &item.kind {
            b"ftyp" if offset == 0 && (16..=4096).contains(&item.size) => ftyp = true,
            b"moov" if ftyp && moov.is_none() && first_fragment.is_none() => moov = Some(item),
            b"moof" if moov.is_some() => {
                first_fragment.get_or_insert(offset);
            }
            b"mdat" if first_fragment.is_some() => {}
            b"styp" | b"sidx" | b"emsg" | b"free" | b"skip" | b"mfra" | b"prft" => {}
            b"pssh" => return Err("DRM/加密媒体无法保存为普通 MP4".into()),
            _ => return Err("录制内容不是完整且受支持的 fMP4".into()),
        }
        offset += item.size;
    }
    let moov = moov.ok_or("录制内容缺少 moov")?;
    let mut offset = first_fragment.ok_or("录制内容没有媒体片段")?;
    let bytes = read_small(&mut file, moov, MAX_METADATA, cancel).await?;
    let tracks = track_set(&bytes, moov.header)?;
    let mut seen = HashSet::new();
    while let Some(item) = read_header(&mut file, offset, length, cancel).await? {
        if item.kind == *b"moof" {
            let data = read_small(&mut file, item, MAX_FRAGMENT, cancel).await?;
            let mut mdats = Vec::new();
            let mut next = offset + item.size;
            while let Some(media) = read_header(&mut file, next, length, cancel).await? {
                if media.kind != *b"mdat" {
                    break;
                }
                if mdats.len() >= 1024 {
                    return Err("片段 mdat 数量超限".into());
                }
                if media.size <= media.header as u64 {
                    return Err("录制片段媒体数据为空".into());
                }
                mdats.push(media);
                next += media.size;
            }
            if mdats.is_empty() {
                return Err("录制末尾片段缺少完整 mdat，原数据已保留".into());
            }
            validate_fragment(&data, item, &mdats, &tracks, &mut seen)?;
            offset = next;
            continue;
        }
        if !matches!(
            &item.kind,
            b"styp" | b"sidx" | b"emsg" | b"free" | b"skip" | b"mfra" | b"prft"
        ) {
            return Err("录制媒体片段顺序无效".into());
        }
        offset += item.size;
    }
    running(cancel)?;
    if seen.len() != tracks.len() {
        return Err("录制声明的音视频轨道未全部包含完整媒体片段".into());
    }
    Ok(())
}

fn track_set(moov: &[u8], header: usize) -> Result<Vec<Metadata>, String> {
    let data = &moov[header..];
    let children = children(data)?;
    let tracks: Vec<_> = children
        .iter()
        .filter(|item| item.kind == *b"trak")
        .copied()
        .collect();
    if tracks.is_empty() || tracks.len() > 2 {
        return Err("fMP4 仅支持单轨或一个视频加一个音频轨".into());
    }
    if children.iter().any(|item| item.kind == *b"pssh") {
        return Err("DRM/加密媒体不受支持".into());
    }
    let mvhd = only(data, b"mvhd")?;
    let mvex = payload(data, only(data, b"mvex")?);
    let defaults = super::boxes::children(mvex)?;
    let mut result = Vec::new();
    for track in tracks {
        let track_data = payload(data, track);
        let tkhd = payload(track_data, only(track_data, b"tkhd")?);
        let id = u32_at(
            tkhd,
            match tkhd.first() {
                Some(0) => 12,
                Some(1) => 20,
                _ => return Err("tkhd 版本无效".into()),
            },
        )?;
        let trex: Vec<_> = defaults
            .iter()
            .filter(|entry| {
                entry.kind == *b"trex" && u32_at(payload(mvex, **entry), 4).ok() == Some(id)
            })
            .copied()
            .collect();
        if trex.len() != 1 {
            return Err("fMP4 轨道的 trex 缺失或重复".into());
        }
        let mut single = Vec::new();
        single.extend_from_slice(&data[mvhd.offset..mvhd.offset + mvhd.size]);
        single.extend_from_slice(&data[track.offset..track.offset + track.size]);
        single.extend(boxed(
            b"mvex",
            &mvex[trex[0].offset..trex[0].offset + trex[0].size],
        )?);
        let meta = metadata::parse(boxed(b"moov", &single)?, 8)?;
        if result
            .iter()
            .any(|saved: &Metadata| saved.id == meta.id || saved.kind == meta.kind)
        {
            return Err("fMP4 音视频轨道或 ID 重复".into());
        }
        result.push(meta);
    }
    if result.len() == 2 && !result.iter().any(|item| item.kind == TrackKind::Video) {
        return Err("双轨 fMP4 必须包含视频和音频".into());
    }
    Ok(result)
}

fn validate_fragment(
    data: &[u8],
    item: BoxHeader,
    mdats: &[BoxHeader],
    tracks: &[Metadata],
    seen: &mut HashSet<u32>,
) -> Result<(), String> {
    let moof = &data[item.header..];
    let children = children(moof)?;
    let mfhd = only(moof, b"mfhd")?;
    let trafs: Vec<_> = children
        .iter()
        .filter(|child| child.kind == *b"traf")
        .copied()
        .collect();
    if trafs.is_empty()
        || trafs.len() > 2
        || children
            .iter()
            .any(|child| !matches!(&child.kind, b"mfhd" | b"traf"))
    {
        return Err("moof 轨道布局无效".into());
    }
    let mut current = HashSet::new();
    for traf in &trafs {
        let traf_data = payload(moof, *traf);
        let tfhd = payload(traf_data, only(traf_data, b"tfhd")?);
        let id = u32_at(tfhd, 4)?;
        let meta = tracks
            .iter()
            .find(|meta| meta.id == id)
            .ok_or("片段引用未声明的轨道")?;
        if !current.insert(id) {
            return Err("同一 moof 含重复轨道".into());
        }
        if trafs.len() > 1 && u32_at(tfhd, 0)? & 0x20001 == 0 {
            return Err("多轨片段使用隐式数据基址，需通过 FFmpeg 正确保存".into());
        }
        let mut single = Vec::new();
        single.extend_from_slice(&moof[mfhd.offset..mfhd.offset + mfhd.size]);
        single.extend_from_slice(&moof[traf.offset..traf.offset + traf.size]);
        let mut single = boxed(b"moof", &single)?;
        fragments::patch(&mut single, 8, item.offset, item.offset, mdats, meta, id, 0)?;
        seen.insert(id);
    }
    Ok(())
}
