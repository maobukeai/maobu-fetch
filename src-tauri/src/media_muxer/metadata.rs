use super::boxes::*;
use std::path::Path;
use tokio::fs::File;
use tokio_util::sync::CancellationToken;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrackKind {
    Video,
    Audio,
}
pub struct Metadata {
    pub kind: TrackKind,
    pub id: u32,
    pub default_size: u32,
    pub moov: Vec<u8>,
    pub header: usize,
}
pub struct Input {
    pub file: File,
    pub length: u64,
    pub moov_offset: u64,
    pub fragments: u64,
    pub metadata: Metadata,
}
impl Input {
    pub async fn open(path: &Path, cancel: &CancellationToken) -> Result<Self, String> {
        ordinary(path).await?;
        running(cancel)?;
        let mut file = File::open(path).await.map_err(io_error)?;
        let length = file.metadata().await.map_err(io_error)?.len();
        let mut offset = 0;
        let mut moov = None;
        let mut fragments = None;
        let mut ftyp = false;
        let mut count = 0;
        while let Some(item) = read_header(&mut file, offset, length, cancel).await? {
            count += 1;
            if count > 1_000_000 {
                return Err("MP4 box 数量超过安全上限".into());
            }
            match &item.kind {
                b"ftyp" if offset == 0 && item.size >= 16 && item.size <= 4096 => ftyp = true,
                b"moov" if ftyp && moov.is_none() && fragments.is_none() => moov = Some(item),
                b"moof" if moov.is_some() => {
                    fragments.get_or_insert(offset);
                }
                b"mdat" if fragments.is_some() => {}
                b"styp" | b"sidx" | b"emsg" | b"free" | b"skip" | b"mfra" | b"prft" => {}
                b"pssh" => return Err("DRM/加密 MP4 不受支持，无法内置合并".into()),
                _ => {
                    return Err(
                        "输入不是受支持的单轨 fragmented MP4（TS/普通 MP4 请使用正确的封装流程）"
                            .into(),
                    )
                }
            }
            offset += item.size;
        }
        let moov = moov.ok_or("MP4 缺少唯一 moov 初始化段")?;
        let fragments = fragments.ok_or("输入没有 moof 媒体片段，不是 fragmented MP4")?;
        let bytes = read_small(&mut file, moov, MAX_METADATA, cancel).await?;
        let metadata = parse(bytes, moov.header)?;
        Ok(Self {
            file,
            length,
            moov_offset: moov.offset,
            fragments,
            metadata,
        })
    }
}

pub(super) fn parse(moov: Vec<u8>, header: usize) -> Result<Metadata, String> {
    let data = &moov[header..];
    let mvhd = payload(data, only(data, b"mvhd")?);
    if !matches!((mvhd.first(), mvhd.len()), (Some(0), 100) | (Some(1), 112)) {
        return Err("MP4 mvhd 版本或长度无效".into());
    }
    let trak = only(data, b"trak")?;
    let track = payload(data, trak);
    if children(data)?.iter().any(|item| item.kind == *b"pssh") {
        return Err("DRM/加密 MP4 不受支持".into());
    }
    if children(track)?
        .iter()
        .any(|item| matches!(&item.kind, b"tref" | b"edts"))
    {
        return Err("暂不支持含跨轨引用或编辑列表的 fMP4，请使用 FFmpeg 合并".into());
    }
    let tkhd = only(track, b"tkhd")?;
    let tkhd_data = payload(track, tkhd);
    let field = match tkhd_data.first() {
        Some(0) => 12,
        Some(1) => 20,
        _ => return Err("不支持的 tkhd 版本".into()),
    };
    let id = u32_at(tkhd_data, field)?;
    if id == 0 {
        return Err("MP4 track_id 不得为 0".into());
    }
    let mdia = payload(track, only(track, b"mdia")?);
    let hdlr = payload(mdia, only(mdia, b"hdlr")?);
    let kind = match hdlr.get(8..12) {
        Some(b"vide") => TrackKind::Video,
        Some(b"soun") => TrackKind::Audio,
        _ => return Err("MP4 轨道不是唯一的视频或音频轨".into()),
    };
    validate_sample_entry(mdia, kind)?;
    let mvex = payload(data, only(data, b"mvex")?);
    let trex = payload(mvex, only(mvex, b"trex")?);
    if u32_at(trex, 0)? != 0 || u32_at(trex, 4)? != id || u32_at(trex, 8)? != 1 || trex.len() != 24
    {
        return Err("MP4 trex 版本、长度或 track_id 无效".into());
    }
    let default_size = u32_at(trex, 16)?;
    Ok(Metadata {
        kind,
        id,
        default_size,
        moov,
        header,
    })
}

fn validate_sample_entry(mdia: &[u8], kind: TrackKind) -> Result<(), String> {
    let minf = payload(mdia, only(mdia, b"minf")?);
    let stbl = payload(minf, only(minf, b"stbl")?);
    let stsd = payload(stbl, only(stbl, b"stsd")?);
    if u32_at(stsd, 0)? != 0 || u32_at(stsd, 4)? != 1 {
        return Err("MP4 必须使用一个明确的采样描述".into());
    }
    let entries = children(stsd.get(8..).ok_or("stsd 截断")?)?;
    if entries.len() != 1 {
        return Err("MP4 采样描述数量无效".into());
    }
    let entry = entries[0];
    if matches!(&entry.kind, b"encv" | b"enca") {
        return Err("DRM/加密采样不受支持，无法内置合并".into());
    }
    let accepted = match kind {
        TrackKind::Video => matches!(
            &entry.kind,
            b"avc1" | b"avc3" | b"hvc1" | b"hev1" | b"vp09" | b"av01"
        ),
        TrackKind::Audio => matches!(&entry.kind, b"mp4a" | b"Opus" | b"ac-3" | b"ec-3" | b"alac"),
    };
    if !accepted {
        return Err("MP4 采样编码不受内置合并支持，请使用 FFmpeg".into());
    }
    let sample = payload(&stsd[8..], entry);
    let prefix = if kind == TrackKind::Video { 78 } else { 28 };
    if sample.len() < prefix || sample.get(6..8) != Some(&[0, 1]) {
        return Err("MP4 采样描述或数据引用无效".into());
    }
    if kind == TrackKind::Audio && sample.get(8..10) != Some(&[0, 0]) {
        return Err("暂不支持扩展版本的音频采样描述，请使用 FFmpeg".into());
    }
    if children(&sample[prefix..])?
        .iter()
        .any(|item| item.kind == *b"sinf")
    {
        return Err("DRM/加密采样不受支持".into());
    }
    if let Some(dinf) = children(minf)?
        .into_iter()
        .find(|item| item.kind == *b"dinf")
    {
        let info = payload(minf, dinf);
        let dref = payload(info, only(info, b"dref")?);
        if u32_at(dref, 0)? != 0 || u32_at(dref, 4)? != 1 {
            return Err("不支持 MP4 外部媒体引用".into());
        }
        let refs = children(dref.get(8..).ok_or("dref 截断")?)?;
        if refs.len() != 1
            || refs[0].kind != *b"url "
            || u32_at(payload(&dref[8..], refs[0]), 0)? != 1
        {
            return Err("不支持 MP4 外部媒体引用".into());
        }
    }
    Ok(())
}

pub fn merged(video: &Metadata, audio: &Metadata) -> Result<Vec<u8>, String> {
    if video.kind != TrackKind::Video || audio.kind != TrackKind::Audio {
        return Err("合并输入必须是明确的一条视频轨和一条音频轨".into());
    }
    let mut body = Vec::new();
    body.try_reserve_exact((2 * MAX_METADATA) as usize)
        .map_err(|_| "MP4 合并元数据内存不足")?;
    let mut defaults = Vec::new();
    for (meta, new_id) in [(video, 1), (audio, 2)] {
        let data = &meta.moov[meta.header..];
        for child in children(data)? {
            match &child.kind {
                b"trak" => {
                    let mut track = data[child.offset..child.offset + child.size].to_vec();
                    let tkhd = only(&track[child.header..], b"tkhd")?;
                    let position = child.header + tkhd.offset + tkhd.header;
                    let id_offset = match track.get(position) {
                        Some(0) => 12,
                        Some(1) => 20,
                        _ => return Err("tkhd 版本无效".into()),
                    };
                    put_u32(&mut track, position + id_offset, new_id)?;
                    body.extend_from_slice(&track);
                }
                b"mvex" => {
                    let mvex = payload(data, child);
                    let trex = only(mvex, b"trex")?;
                    let mut item = mvex[trex.offset..trex.offset + trex.size].to_vec();
                    put_u32(&mut item, trex.header + 4, new_id)?;
                    defaults.extend_from_slice(&item);
                }
                b"mvhd" if new_id == 1 => {
                    let mut mvhd = data[child.offset..child.offset + child.size].to_vec();
                    let payload = child.header;
                    match mvhd.get(payload) {
                        Some(0) => put_u32(&mut mvhd, payload + 16, 0)?,
                        Some(1) => put_u64(&mut mvhd, payload + 24, 0)?,
                        _ => return Err("mvhd 版本无效".into()),
                    };
                    let next = mvhd.len().checked_sub(4).ok_or("mvhd 截断")?;
                    put_u32(&mut mvhd, next, 3)?;
                    body.extend_from_slice(&mvhd);
                }
                b"udta" | b"meta" if new_id == 1 => {
                    body.extend_from_slice(&data[child.offset..child.offset + child.size])
                }
                b"mvhd" | b"udta" | b"meta" | b"free" | b"skip" => {}
                _ => return Err("moov 含内置合并暂不支持的关联元数据，请使用 FFmpeg".into()),
            }
        }
    }
    body.extend_from_slice(&boxed(b"mvex", &defaults)?);
    boxed(b"moov", &body)
}
