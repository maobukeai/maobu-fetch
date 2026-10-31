//! Legacy parser helper regression surface (test-only).
/// MP4 box 的简化表示（顶级或嵌套）。
#[derive(Clone, Copy, Debug)]
pub(super) struct Mp4Box {
    /// box 在文件/父 box payload 中的偏移（从 payload 起算，不含 8 字节 header）。
    pub offset: usize,
    /// box 总大小（含 header）。
    pub size: usize,
    /// box 类型（4 字节 ASCII）。
    pub box_type: [u8; 4],
}

/// 大端序读取 u32。
pub(super) fn read_u32_be(data: &[u8], offset: usize) -> Result<u32, String> {
    if offset + 4 > data.len() {
        return Err(format!(
            "读取 u32 越界：offset={offset}, len={}",
            data.len()
        ));
    }
    Ok(u32::from_be_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ]))
}

/// 大端序写入 u32。
pub(super) fn write_u32_be(data: &mut [u8], offset: usize, value: u32) -> Result<(), String> {
    if offset + 4 > data.len() {
        return Err(format!(
            "写入 u32 越界：offset={offset}, len={}",
            data.len()
        ));
    }
    data[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
    Ok(())
}

/// 解析一个 box 的 header，返回 (box_size, header_size, box_type)。
///
/// - `box_size`：box 总大小（含 header）。若为 0 表示文件末尾；若为 1 表示使用 64 位扩展大小。
/// - `header_size`：header 字节数（8 或 16）。
/// - `box_type`：4 字节类型。
pub(super) fn parse_box_header(
    data: &[u8],
    offset: usize,
) -> Result<(usize, usize, [u8; 4]), String> {
    if offset + 8 > data.len() {
        return Err(format!(
            "box header 越界：offset={offset}, len={}",
            data.len()
        ));
    }
    let size = read_u32_be(data, offset)? as usize;
    let mut box_type = [0u8; 4];
    box_type.copy_from_slice(&data[offset + 4..offset + 8]);
    if size == 1 {
        // 64 位扩展大小：header 为 16 字节（4 size + 4 type + 8 extended size）
        if offset + 16 > data.len() {
            return Err(format!(
                "扩展 box header 越界：offset={offset}, len={}",
                data.len()
            ));
        }
        let extended = u64::from_be_bytes([
            data[offset + 8],
            data[offset + 9],
            data[offset + 10],
            data[offset + 11],
            data[offset + 12],
            data[offset + 13],
            data[offset + 14],
            data[offset + 15],
        ]);
        Ok((extended as usize, 16, box_type))
    } else if size == 0 {
        // size == 0 表示 box 延伸到文件末尾
        Ok((data.len() - offset, 8, box_type))
    } else {
        Ok((size, 8, box_type))
    }
}

/// 遍历顶级 box，返回所有顶级 box 的偏移、大小和类型。
pub(super) fn parse_top_level_boxes(data: &[u8]) -> Result<Vec<Mp4Box>, String> {
    let mut boxes = Vec::new();
    let mut offset = 0usize;
    while offset < data.len() {
        let (size, _header_size, box_type) = parse_box_header(data, offset)?;
        if size == 0 {
            break;
        }
        boxes.push(Mp4Box {
            offset,
            size,
            box_type,
        });
        offset += size;
    }
    Ok(boxes)
}

/// 在 box payload 中递归查找指定类型的子 box。
///
/// `payload_offset` 是父 box 的 payload 起始位置（不含 header）。
/// 返回所有匹配的子 box。
pub(super) fn find_child_boxes(
    payload: &[u8],
    target_type: &[u8; 4],
) -> Result<Vec<Mp4Box>, String> {
    let mut found = Vec::new();
    let mut offset = 0usize;
    while offset < payload.len() {
        let (size, _header_size, box_type) = parse_box_header(payload, offset)?;
        if size == 0 {
            break;
        }
        if &box_type == target_type {
            found.push(Mp4Box {
                offset,
                size,
                box_type,
            });
        }
        offset += size;
    }
    Ok(found)
}

/// 在 moov box 的 payload 中查找所有 trak box。
pub(super) fn find_trak_boxes_in_moov(moov_payload: &[u8]) -> Result<Vec<Mp4Box>, String> {
    find_child_boxes(moov_payload, b"trak")
}

/// 修改 trak box 中 tkhd 的 track_id。
///
/// `trak_bytes` 是完整的 trak box 字节（含 8 字节 header）。
///
/// tkhd 是 FullBox（version + flags + payload）：
/// - version 0: creation_time(4) + modification_time(4) + track_id(4) + reserved(4) + duration(4)...
/// - version 1: creation_time(8) + modification_time(8) + track_id(4) + reserved(4) + duration(8)...
///
/// track_id 偏移（从 tkhd box 起算）：
/// - version 0: 8(header) + 4(ver/flags) + 4(creation) + 4(modification) = 20
/// - version 1: 8(header) + 4(ver/flags) + 8(creation) + 8(modification) = 28
pub(super) fn patch_tkhd_track_id(trak_bytes: &mut [u8], new_id: u32) -> Result<(), String> {
    if trak_bytes.len() < 8 {
        return Err("trak box 过小".into());
    }
    // trak payload 从偏移 8 开始（跳过 size+type header）
    let trak_payload = &trak_bytes[8..];
    let tkhd_boxes = find_child_boxes(trak_payload, b"tkhd")?;
    if tkhd_boxes.is_empty() {
        return Err("trak 中未找到 tkhd box".into());
    }
    let tkhd = tkhd_boxes[0];
    // tkhd.offset 是相对于 trak_payload 的偏移，需要 +8 转换为相对于 trak_bytes 的偏移
    let tkhd_abs_offset = tkhd.offset + 8;
    // tkhd header 8 字节，之后是 version(1) + flags(3)
    let version_offset = tkhd_abs_offset + 8;
    if version_offset + 4 > trak_bytes.len() {
        return Err("tkhd version/flags 越界".into());
    }
    let version = trak_bytes[version_offset];
    let track_id_offset = match version {
        0 => tkhd_abs_offset + 20,
        1 => tkhd_abs_offset + 28,
        v => return Err(format!("不支持的 tkhd version：{v}")),
    };
    write_u32_be(trak_bytes, track_id_offset, new_id)
}

/// 修改 trex box 中的 track_id。
///
/// trex 是 FullBox，payload 结构：
/// version/flags(4) + track_id(4) + default_sample_description_index(4) + ...
pub(super) fn patch_trex_track_id(mvex_payload: &mut [u8], new_id: u32) -> Result<(), String> {
    let trex_boxes = find_child_boxes(mvex_payload, b"trex")?;
    if trex_boxes.is_empty() {
        // mvex 中没有 trex 是合法的（虽然不常见），跳过。
        return Ok(());
    }
    for trex in trex_boxes {
        // trex header 8 字节，之后是 version(1) + flags(3)，然后是 track_id(4)
        let track_id_offset = trex.offset + 8 + 4;
        write_u32_be(mvex_payload, track_id_offset, new_id)?;
    }
    Ok(())
}

/// 修改 moof box 中 tfhd 的 track_id。
///
/// fMP4 结构：moof → traf → tfhd（tfhd 不是 moof 的直接子节点）。
/// 本函数先找 moof 内的所有 traf，再在每个 traf 内找 tfhd 并修改 track_id。
///
/// tfhd 是 FullBox，payload 结构：
/// version/flags(4) + track_id(4) + ...
pub(super) fn patch_moof_track_id(moof_payload: &mut [u8], new_id: u32) -> Result<(), String> {
    let traf_boxes = find_child_boxes(moof_payload, b"traf")?;
    if traf_boxes.is_empty() {
        return Err("moof 中未找到 traf box".into());
    }
    for traf in traf_boxes {
        // traf payload 是相对 moof_payload 的子区间
        let traf_start = traf.offset + 8;
        let traf_end = traf.offset + traf.size;
        if traf_end > moof_payload.len() {
            return Err("traf box 越界".into());
        }
        let tfhd_boxes = find_child_boxes(&moof_payload[traf_start..traf_end], b"tfhd")?;
        if tfhd_boxes.is_empty() {
            // 某些 traf 可能不含 tfhd（异常情况），跳过
            continue;
        }
        for tfhd in tfhd_boxes {
            // tfhd header 8 字节，之后是 version(1) + flags(3)，然后是 track_id(4)
            // tfhd.offset 是相对 traf payload 的偏移，需转换为相对 moof_payload 的偏移
            let track_id_offset = traf_start + tfhd.offset + 8 + 4;
            write_u32_be(moof_payload, track_id_offset, new_id)?;
        }
    }
    Ok(())
}

/// 找到第一个 fragment 的偏移（第一个 styp 或 moof box 的位置）。
///
/// fMP4 结构：ftyp + [free] + moov + (styp + moof + mdat) * N
/// 第一个 fragment 起始于 moov 之后的第一个 box。
pub(super) fn find_first_fragment_offset(boxes: &[Mp4Box]) -> Result<usize, String> {
    let moov_idx = boxes
        .iter()
        .position(|b| &b.box_type == b"moov")
        .ok_or("文件中未找到 moov box")?;
    let moov = boxes[moov_idx];
    let next_offset = moov.offset + moov.size;
    Ok(next_offset)
}

/// 验证输入文件是 fragmented MP4。
///
/// 检查：必须包含 ftyp + moov，且 moov 之后存在 styp 或 moof box。
pub(super) fn validate_fragmented_mp4(boxes: &[Mp4Box]) -> Result<(), String> {
    let has_ftyp = boxes.iter().any(|b| &b.box_type == b"ftyp");
    if !has_ftyp {
        return Err("文件缺少 ftyp box，不是有效 MP4".into());
    }
    let moov = boxes
        .iter()
        .find(|b| &b.box_type == b"moov")
        .ok_or("文件缺少 moov box")?;
    let after_moov = boxes.iter().find(|b| b.offset >= moov.offset + moov.size);
    match after_moov {
        Some(b) => {
            if &b.box_type != b"styp" && &b.box_type != b"moof" {
                return Err(format!(
                    "moov 之后不是 styp/moof（而是 {}），可能不是 fragmented MP4",
                    String::from_utf8_lossy(&b.box_type)
                ));
            }
        }
        None => return Err("moov 之后没有任何 box，不是 fragmented MP4".into()),
    }
    Ok(())
}

/// 从音频文件的 moov payload 中提取 trak box（含其完整字节），并修改 track_id 为 2。
///
/// 返回修改后的 trak box 完整字节（含 8 字节 header）。
pub(super) fn extract_and_patch_audio_trak(
    audio_moov_payload: &[u8],
    new_id: u32,
) -> Result<Vec<u8>, String> {
    let trak_boxes = find_trak_boxes_in_moov(audio_moov_payload)?;
    if trak_boxes.is_empty() {
        return Err("音频 moov 中未找到 trak box".into());
    }
    if trak_boxes.len() > 1 {
        return Err(format!(
            "音频 moov 包含 {} 个 trak，本合并器仅支持单 track 输入",
            trak_boxes.len()
        ));
    }
    let trak = trak_boxes[0];
    let mut trak_bytes = audio_moov_payload[trak.offset..trak.offset + trak.size].to_vec();

    // 修改 tkhd 中的 track_id
    patch_tkhd_track_id(&mut trak_bytes, new_id)?;

    Ok(trak_bytes)
}

/// 修改音频 fragments 区域中所有 moof 的 tfhd track_id。
///
/// `fragments_data` 是从第一个 fragment 开始到文件末尾的字节。
pub(super) fn patch_audio_fragments_track_id(
    fragments_data: &mut [u8],
    new_id: u32,
) -> Result<(), String> {
    let mut offset = 0usize;
    while offset < fragments_data.len() {
        let (size, _header_size, box_type) = parse_box_header(fragments_data, offset)?;
        if size == 0 {
            break;
        }
        if &box_type == b"moof" {
            // moof payload 从 offset+8 开始（假设无扩展大小）
            let moof_end = offset + size;
            if moof_end > fragments_data.len() {
                break;
            }
            let moof_payload = &mut fragments_data[offset + 8..moof_end];
            patch_moof_track_id(moof_payload, new_id)?;
        }
        offset += size;
    }
    Ok(())
}

pub(super) fn build_merged_moov_with_extras(
    video_moov_bytes: &[u8],
    audio_trak_bytes: &[u8],
    audio_mvex_bytes: Option<&[u8]>,
) -> Result<Vec<u8>, String> {
    if video_moov_bytes.len() < 8 {
        return Err("视频 moov box 过小".into());
    }
    if &video_moov_bytes[4..8] != b"moov" {
        return Err("传入的字节不是 moov box".into());
    }
    let video_moov_payload = &video_moov_bytes[8..];

    let mut new_payload_size = video_moov_payload.len() + audio_trak_bytes.len();
    if let Some(mvex) = audio_mvex_bytes {
        new_payload_size += mvex.len();
    }
    let new_moov_size = 8 + new_payload_size;

    let mut new_moov = Vec::with_capacity(new_moov_size);
    new_moov.extend_from_slice(&(new_moov_size as u32).to_be_bytes());
    new_moov.extend_from_slice(b"moov");
    new_moov.extend_from_slice(video_moov_payload);
    new_moov.extend_from_slice(audio_trak_bytes);
    if let Some(mvex) = audio_mvex_bytes {
        new_moov.extend_from_slice(mvex);
    }

    Ok(new_moov)
}
