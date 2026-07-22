//! 纯 Rust 实现的 fragmented MP4 合并器。
//!
//! 用于 Twitter/X 等平台：yt-dlp 在没有 FFmpeg 时下载视频和音频为两个独立的
//! fragmented MP4 文件，本模块将它们合并为单个 fMP4 文件，包含两个 track
//! （视频 track_id=1，音频 track_id=2）。
//!
//! ## 设计目标
//!
//! - 不引入 `mp4` / `symphonia` 等外部 crate（AGENTS.md §8：可用标准库或现有
//!   依赖完成时不得新增依赖）。
//! - 纯字节级 box 解析与重组，最小化内存占用（流式读取顶级 box，仅对 moov 与
//!   fragments 中的 track_id 字段做就地修改）。
//! - 仅处理 fragmented MP4（HLS 切片下载的标准结构：`ftyp + moov + (styp + moof + mdat) * N`）。
//!   progressive MP4 不在此场景范围内（yt-dlp 无 FFmpeg 时对 HLS 输出始终是 fMP4）。
//!
//! ## 工作原理
//!
//! 1. 解析两个文件的顶级 box：`ftyp` / `moov` / `styp` / `moof` / `mdat` / `free` 等。
//! 2. 从音频文件的 `moov` 中提取 `trak` box，递归修改其中 `tkhd` 与 `trex` 的
//!    `track_id`（从 1 改为 2）。
//! 3. 修改音频文件每个 `moof` 中 `tfhd` 的 `track_id`（从 1 改为 2）。
//! 4. 构建新 `moov`：视频 `moov` 的所有子 box + 修改后的音频 `trak`，重算 `moov` size。
//! 5. 写出新文件：视频 `ftyp` + 视频 `ftyp` 到 `moov` 之间的 box（如 `free`）+ 新 `moov`
//!    + 视频所有 `(styp + moof + mdat)` fragments + 音频所有 fragments。
//!
//! ## 局限性
//!
//! - 仅支持 fMP4 输入（progressive MP4 不支持，会返回错误）。
//! - 假设视频文件所有 track 的 `track_id` 都是 1，音频文件同样。
//!   若输入文件的 `track_id` 不是 1，合并后的文件可能损坏。
//! - 不修改 `mvhd` 的 `duration`（fMP4 中通常为 0，由 fragment 累计计算）。
//! - 不处理 `sidx`（Segment Index Box），Twitter HLS 切片未包含。

use std::path::Path;

/// MP4 box 的简化表示（顶级或嵌套）。
#[derive(Clone, Copy, Debug)]
struct Mp4Box {
    /// box 在文件/父 box payload 中的偏移（从 payload 起算，不含 8 字节 header）。
    offset: usize,
    /// box 总大小（含 header）。
    size: usize,
    /// box 类型（4 字节 ASCII）。
    box_type: [u8; 4],
}

/// 大端序读取 u32。
fn read_u32_be(data: &[u8], offset: usize) -> Result<u32, String> {
    if offset + 4 > data.len() {
        return Err(format!("读取 u32 越界：offset={offset}, len={}", data.len()));
    }
    Ok(u32::from_be_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ]))
}

/// 大端序写入 u32。
fn write_u32_be(data: &mut [u8], offset: usize, value: u32) -> Result<(), String> {
    if offset + 4 > data.len() {
        return Err(format!("写入 u32 越界：offset={offset}, len={}", data.len()));
    }
    data[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
    Ok(())
}

/// 解析一个 box 的 header，返回 (box_size, header_size, box_type)。
///
/// - `box_size`：box 总大小（含 header）。若为 0 表示文件末尾；若为 1 表示使用 64 位扩展大小。
/// - `header_size`：header 字节数（8 或 16）。
/// - `box_type`：4 字节类型。
fn parse_box_header(data: &[u8], offset: usize) -> Result<(usize, usize, [u8; 4]), String> {
    if offset + 8 > data.len() {
        return Err(format!("box header 越界：offset={offset}, len={}", data.len()));
    }
    let size = read_u32_be(data, offset)? as usize;
    let mut box_type = [0u8; 4];
    box_type.copy_from_slice(&data[offset + 4..offset + 8]);
    if size == 1 {
        // 64 位扩展大小：header 为 16 字节（4 size + 4 type + 8 extended size）
        if offset + 16 > data.len() {
            return Err(format!("扩展 box header 越界：offset={offset}, len={}", data.len()));
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
fn parse_top_level_boxes(data: &[u8]) -> Result<Vec<Mp4Box>, String> {
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
fn find_child_boxes(payload: &[u8], target_type: &[u8; 4]) -> Result<Vec<Mp4Box>, String> {
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
fn find_trak_boxes_in_moov(moov_payload: &[u8]) -> Result<Vec<Mp4Box>, String> {
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
fn patch_tkhd_track_id(trak_bytes: &mut [u8], new_id: u32) -> Result<(), String> {
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
fn patch_trex_track_id(mvex_payload: &mut [u8], new_id: u32) -> Result<(), String> {
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
fn patch_moof_track_id(moof_payload: &mut [u8], new_id: u32) -> Result<(), String> {
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
fn find_first_fragment_offset(boxes: &[Mp4Box]) -> Result<usize, String> {
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
fn validate_fragmented_mp4(boxes: &[Mp4Box]) -> Result<(), String> {
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
fn extract_and_patch_audio_trak(audio_moov_payload: &[u8], new_id: u32) -> Result<Vec<u8>, String> {
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
fn patch_audio_fragments_track_id(fragments_data: &mut [u8], new_id: u32) -> Result<(), String> {
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

/// 合并两个 fragmented MP4 文件（视频轨 + 音频轨）为单个 fMP4 文件。
///
/// ## 参数
///
/// - `video_path`：仅含视频轨的 fMP4 文件路径（来自 yt-dlp 无 FFmpeg 时的输出）。
/// - `audio_path`：仅含音频轨的 fMP4 文件路径。
/// - `output_path`：合并后的输出文件路径。
///
/// ## 错误
///
/// - 输入文件不是 fMP4 结构。
/// - 输入文件包含多个 track（本合并器仅支持单 track 输入）。
/// - box 结构损坏或越界。
/// - I/O 错误。
///
/// ## 安全约束（AGENTS.md §3 / §7）
///
/// - 不使用 `unwrap()` / `expect()` 处理可恢复错误。
/// - 输出文件先写入临时文件，合并成功后原子重命名（避免半成品暴露为完成文件）。
/// - 不修改输入文件。
pub async fn merge_fragmented_mp4(
    video_path: &Path,
    audio_path: &Path,
    output_path: &Path,
) -> Result<(), String> {
    // 读取两个输入文件
    let video_data = tokio::fs::read(video_path)
        .await
        .map_err(|e| format!("读取视频文件失败：{e}"))?;
    let audio_data = tokio::fs::read(audio_path)
        .await
        .map_err(|e| format!("读取音频文件失败：{e}"))?;

    // 解析顶级 box
    let video_boxes = parse_top_level_boxes(&video_data)?;
    let audio_boxes = parse_top_level_boxes(&audio_data)?;

    // 验证 fMP4 结构
    validate_fragmented_mp4(&video_boxes).map_err(|e| format!("视频文件验证失败：{e}"))?;
    validate_fragmented_mp4(&audio_boxes).map_err(|e| format!("音频文件验证失败：{e}"))?;

    // 找到视频的 ftyp 区间（ftyp 及 moov 之前的所有 box，如 free）和 moov
    let video_moov = video_boxes
        .iter()
        .find(|b| &b.box_type == b"moov")
        .ok_or("视频文件缺少 moov")?;
    let video_first_frag_offset = find_first_fragment_offset(&video_boxes)?;

    // 找到音频的 moov 和 fragments 区间
    let audio_moov = audio_boxes
        .iter()
        .find(|b| &b.box_type == b"moov")
        .ok_or("音频文件缺少 moov")?;
    let audio_first_frag_offset = find_first_fragment_offset(&audio_boxes)?;

    // 处理音频 moov：提取 trak 并修改 track_id 为 2
    let audio_moov_bytes = &audio_data[audio_moov.offset..audio_moov.offset + audio_moov.size];
    let audio_moov_payload = &audio_moov_bytes[8..];
    let audio_trak_bytes = extract_and_patch_audio_trak(audio_moov_payload, 2)?;

    // 提取音频 mvex 并修改其中 trex 的 track_id 为 2。
    // mvex 必须包含在新 moov 中，否则播放器无法解析音频 fragment 的默认 sample 属性。
    let audio_mvex_bytes = {
        let mvex_boxes = find_child_boxes(audio_moov_payload, b"mvex")?;
        if mvex_boxes.is_empty() {
            None
        } else {
            let mvex = mvex_boxes[0];
            let mut mvex_bytes = audio_moov_payload[mvex.offset..mvex.offset + mvex.size].to_vec();
            // 修改 mvex payload 中的 trex track_id
            let mvex_payload = &mut mvex_bytes[8..];
            patch_trex_track_id(mvex_payload, 2)?;
            Some(mvex_bytes)
        }
    };

    // 构建新 moov
    let video_moov_bytes =
        &video_data[video_moov.offset..video_moov.offset + video_moov.size];
    let new_moov = build_merged_moov_with_extras(
        video_moov_bytes,
        &audio_trak_bytes,
        audio_mvex_bytes.as_deref(),
    )?;

    // 处理音频 fragments：修改所有 moof 中 tfhd 的 track_id 为 2
    let mut audio_fragments = audio_data[audio_first_frag_offset..].to_vec();
    patch_audio_fragments_track_id(&mut audio_fragments, 2)?;

    // 拼接输出文件：
    // 视频 ftyp + 视频 (ftyp 到 moov 之间的 box，如 free) + 新 moov + 视频 fragments + 音频 fragments
    let mut output = Vec::with_capacity(
        video_moov.offset + new_moov.len()
            + (video_data.len() - video_first_frag_offset)
            + audio_fragments.len(),
    );
    output.extend_from_slice(&video_data[..video_moov.offset]); // ftyp + free 等
    output.extend_from_slice(&new_moov);
    output.extend_from_slice(&video_data[video_first_frag_offset..]); // 视频 fragments
    output.extend_from_slice(&audio_fragments); // 音频 fragments

    // 原子写入：先写临时文件，再重命名（AGENTS.md §3：合并必须写入临时文件，
    // 完成长度或校验通过后再原子替换目标文件）
    let temp_path = output_path.with_extension("merging.tmp");
    tokio::fs::write(&temp_path, &output)
        .await
        .map_err(|e| format!("写入合并临时文件失败：{e}"))?;

    // 验证临时文件大小（最低限度完整性检查）
    let temp_meta = tokio::fs::metadata(&temp_path)
        .await
        .map_err(|e| format!("读取临时文件元数据失败：{e}"))?;
    if temp_meta.len() as usize != output.len() {
        let _ = tokio::fs::remove_file(&temp_path).await;
        return Err(format!(
            "临时文件大小不匹配：预期 {}，实际 {}",
            output.len(),
            temp_meta.len()
        ));
    }

    // 原子重命名
    tokio::fs::rename(&temp_path, output_path)
        .await
        .map_err(|e| format!("原子替换目标文件失败：{e}"))?;

    Ok(())
}

/// 构建合并后的新 moov box 字节（含音频 trak 与可选的音频 mvex）。
///
/// 新 moov = 视频 moov 的所有子 box + 修改后的音频 trak + 修改后的音频 mvex（可选）。
fn build_merged_moov_with_extras(
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 构造一个最小化的 fMP4 字节用于测试 box 解析。
    ///
    /// 结构：ftyp + moov(mvhd + trak(tkhd) + mvex(trex)) + styp + moof(traf(tfhd)) + mdat
    fn build_minimal_fmp4(track_id: u32) -> Vec<u8> {
        let mut data = Vec::new();

        // ftyp box: size=16, type=ftyp, major_brand=isom, minor_version=0
        data.extend_from_slice(&16u32.to_be_bytes());
        data.extend_from_slice(b"ftyp");
        data.extend_from_slice(b"isom");
        data.extend_from_slice(&0u32.to_be_bytes());

        // 先构造 moov payload，最后再写 moov header
        let mut moov_payload = Vec::new();

        // mvhd box (version 0): size=108
        let mvhd_size = 108usize;
        moov_payload.extend_from_slice(&(mvhd_size as u32).to_be_bytes());
        moov_payload.extend_from_slice(b"mvhd");
        moov_payload.extend_from_slice(&0u32.to_be_bytes()); // version + flags
        moov_payload.extend_from_slice(&vec![0u8; mvhd_size - 12]);

        // trak box: header + tkhd
        let tkhd_size = 92usize; // version 0 tkhd size
        let trak_size = 8 + tkhd_size;
        moov_payload.extend_from_slice(&(trak_size as u32).to_be_bytes());
        moov_payload.extend_from_slice(b"trak");

        // tkhd box (version 0)
        moov_payload.extend_from_slice(&(tkhd_size as u32).to_be_bytes());
        moov_payload.extend_from_slice(b"tkhd");
        moov_payload.extend_from_slice(&0u32.to_be_bytes()); // version=0 + flags
        moov_payload.extend_from_slice(&vec![0u8; 8]); // creation_time + modification_time
        moov_payload.extend_from_slice(&track_id.to_be_bytes()); // track_id
        moov_payload.extend_from_slice(&vec![0u8; tkhd_size - 24]); // reserved + duration + ...

        // mvex box: header + trex
        let trex_size = 32usize;
        let mvex_size = 8 + trex_size;
        moov_payload.extend_from_slice(&(mvex_size as u32).to_be_bytes());
        moov_payload.extend_from_slice(b"mvex");

        // trex box
        moov_payload.extend_from_slice(&(trex_size as u32).to_be_bytes());
        moov_payload.extend_from_slice(b"trex");
        moov_payload.extend_from_slice(&0u32.to_be_bytes()); // version + flags
        moov_payload.extend_from_slice(&track_id.to_be_bytes()); // track_id
        moov_payload.extend_from_slice(&vec![0u8; trex_size - 16]); // 其余字段

        // 写 moov header
        let moov_size = 8 + moov_payload.len();
        data.extend_from_slice(&(moov_size as u32).to_be_bytes());
        data.extend_from_slice(b"moov");
        data.extend_from_slice(&moov_payload);

        // styp box: size=16
        data.extend_from_slice(&16u32.to_be_bytes());
        data.extend_from_slice(b"styp");
        data.extend_from_slice(b"msdh");
        data.extend_from_slice(&0u32.to_be_bytes());

        // moof box: header + traf + tfhd（真实 fMP4 结构：moof → traf → tfhd）
        let tfhd_size = 16usize;
        let traf_size = 8 + tfhd_size;
        let moof_size = 8 + traf_size;
        data.extend_from_slice(&(moof_size as u32).to_be_bytes());
        data.extend_from_slice(b"moof");

        // traf box
        data.extend_from_slice(&(traf_size as u32).to_be_bytes());
        data.extend_from_slice(b"traf");

        // tfhd box
        data.extend_from_slice(&(tfhd_size as u32).to_be_bytes());
        data.extend_from_slice(b"tfhd");
        data.extend_from_slice(&0u32.to_be_bytes()); // version + flags
        data.extend_from_slice(&track_id.to_be_bytes()); // track_id

        // mdat box: size=8
        data.extend_from_slice(&8u32.to_be_bytes());
        data.extend_from_slice(b"mdat");

        data
    }

    #[test]
    fn parse_top_level_boxes_recognizes_fmp4_structure() {
        let data = build_minimal_fmp4(1);
        let boxes = parse_top_level_boxes(&data).expect("应成功解析");
        assert_eq!(boxes.len(), 5, "应有 5 个顶级 box");
        assert_eq!(&boxes[0].box_type, b"ftyp");
        assert_eq!(&boxes[1].box_type, b"moov");
        assert_eq!(&boxes[2].box_type, b"styp");
        assert_eq!(&boxes[3].box_type, b"moof");
        assert_eq!(&boxes[4].box_type, b"mdat");
    }

    #[test]
    fn validate_fragmented_mp4_accepts_valid_input() {
        let data = build_minimal_fmp4(1);
        let boxes = parse_top_level_boxes(&data).expect("应成功解析");
        validate_fragmented_mp4(&boxes).expect("应通过验证");
    }

    #[test]
    fn validate_fragmented_mp4_rejects_missing_ftyp() {
        let mut data = build_minimal_fmp4(1);
        // 破坏 ftyp 类型
        data[4..8].copy_from_slice(b"XXXX");
        let boxes = parse_top_level_boxes(&data).expect("应成功解析");
        let err = validate_fragmented_mp4(&boxes).expect_err("应拒绝");
        assert!(err.contains("ftyp"), "错误信息应提到 ftyp：{err}");
    }

    #[test]
    fn patch_tkhd_track_id_modifies_version_0() {
        let data = build_minimal_fmp4(1);
        let boxes = parse_top_level_boxes(&data).expect("应成功解析");
        let moov = boxes.iter().find(|b| &b.box_type == b"moov").unwrap();
        let moov_bytes = &data[moov.offset..moov.offset + moov.size];
        let mut moov_payload = moov_bytes[8..].to_vec();

        let trak_boxes = find_trak_boxes_in_moov(&moov_payload).expect("应找到 trak");
        let trak = trak_boxes[0];
        let mut trak_bytes = moov_payload[trak.offset..trak.offset + trak.size].to_vec();

        patch_tkhd_track_id(&mut trak_bytes, 2).expect("应成功修改");

        // 验证 track_id 已改为 2
        // trak_bytes 是完整 trak box（含 8 字节 header），tkhd 在 payload 中
        let trak_payload = &trak_bytes[8..];
        let new_tkhd_boxes = find_child_boxes(trak_payload, b"tkhd").unwrap();
        let tkhd = new_tkhd_boxes[0];
        // version 0: track_id 偏移（从 tkhd box 起算）= 20
        let track_id = read_u32_be(trak_payload, tkhd.offset + 20).unwrap();
        assert_eq!(track_id, 2, "track_id 应为 2");
    }

    #[test]
    fn patch_trex_track_id_modifies_correctly() {
        let data = build_minimal_fmp4(1);
        let boxes = parse_top_level_boxes(&data).expect("应成功解析");
        let moov = boxes.iter().find(|b| &b.box_type == b"moov").unwrap();
        let moov_bytes = &data[moov.offset..moov.offset + moov.size];
        let mut moov_payload = moov_bytes[8..].to_vec();

        let mvex_boxes = find_child_boxes(&moov_payload, b"mvex").unwrap();
        let mvex = mvex_boxes[0];
        let mvex_payload = &mut moov_payload[mvex.offset + 8..mvex.offset + mvex.size];
        patch_trex_track_id(mvex_payload, 2).expect("应成功修改");

        // 验证 trex track_id 已改为 2
        let trex_boxes = find_child_boxes(mvex_payload, b"trex").unwrap();
        let trex = trex_boxes[0];
        let track_id = read_u32_be(mvex_payload, trex.offset + 8 + 4).unwrap();
        assert_eq!(track_id, 2, "trex track_id 应为 2");
    }

    #[test]
    fn patch_moof_track_id_modifies_tfhd() {
        let data = build_minimal_fmp4(1);
        let boxes = parse_top_level_boxes(&data).expect("应成功解析");
        let moof = boxes.iter().find(|b| &b.box_type == b"moof").unwrap();
        let mut moof_bytes = data[moof.offset..moof.offset + moof.size].to_vec();
        let moof_payload = &mut moof_bytes[8..];

        patch_moof_track_id(moof_payload, 2).expect("应成功修改");

        // 验证 tfhd track_id 已改为 2（tfhd 在 traf 内）
        let traf_boxes = find_child_boxes(moof_payload, b"traf").unwrap();
        let traf = traf_boxes[0];
        let traf_payload = &moof_payload[traf.offset + 8..traf.offset + traf.size];
        let tfhd_boxes = find_child_boxes(traf_payload, b"tfhd").unwrap();
        let tfhd = tfhd_boxes[0];
        let track_id = read_u32_be(traf_payload, tfhd.offset + 8 + 4).unwrap();
        assert_eq!(track_id, 2, "tfhd track_id 应为 2");
    }

    #[test]
    fn extract_and_patch_audio_trak_returns_modified_bytes() {
        let data = build_minimal_fmp4(1);
        let boxes = parse_top_level_boxes(&data).expect("应成功解析");
        let moov = boxes.iter().find(|b| &b.box_type == b"moov").unwrap();
        let moov_bytes = &data[moov.offset..moov.offset + moov.size];
        let moov_payload = &moov_bytes[8..];

        let trak_bytes = extract_and_patch_audio_trak(moov_payload, 2).expect("应成功提取");

        // 验证 trak box 类型
        assert_eq!(&trak_bytes[4..8], b"trak", "应为 trak box");
        // 验证 track_id 已改为 2（trak_bytes 含 8 字节 header，tkhd 在 payload 中）
        let trak_payload = &trak_bytes[8..];
        let tkhd_boxes = find_child_boxes(trak_payload, b"tkhd").unwrap();
        let tkhd = tkhd_boxes[0];
        let track_id = read_u32_be(trak_payload, tkhd.offset + 20).unwrap();
        assert_eq!(track_id, 2, "提取的 trak 中 track_id 应为 2");
    }

    #[test]
    fn build_merged_moov_with_extras_combines_correctly() {
        let video_data = build_minimal_fmp4(1);
        let video_boxes = parse_top_level_boxes(&video_data).unwrap();
        let video_moov = video_boxes
            .iter()
            .find(|b| &b.box_type == b"moov")
            .unwrap();
        let video_moov_bytes =
            &video_data[video_moov.offset..video_moov.offset + video_moov.size];

        let audio_data = build_minimal_fmp4(1);
        let audio_boxes = parse_top_level_boxes(&audio_data).unwrap();
        let audio_moov = audio_boxes
            .iter()
            .find(|b| &b.box_type == b"moov")
            .unwrap();
        let audio_moov_bytes =
            &audio_data[audio_moov.offset..audio_moov.offset + audio_moov.size];
        let audio_moov_payload = &audio_moov_bytes[8..];
        let audio_trak_bytes =
            extract_and_patch_audio_trak(audio_moov_payload, 2).unwrap();

        // 提取音频 mvex
        let audio_mvex_bytes = {
            let mvex_boxes = find_child_boxes(audio_moov_payload, b"mvex").unwrap();
            let mvex = mvex_boxes[0];
            let mut bytes = audio_moov_payload[mvex.offset..mvex.offset + mvex.size].to_vec();
            let payload = &mut bytes[8..];
            patch_trex_track_id(payload, 2).unwrap();
            bytes
        };

        let new_moov = build_merged_moov_with_extras(
            video_moov_bytes,
            &audio_trak_bytes,
            Some(&audio_mvex_bytes),
        )
        .expect("应成功构建");

        // 验证 moov header
        assert_eq!(&new_moov[4..8], b"moov");
        let declared_size = read_u32_be(&new_moov, 0).unwrap() as usize;
        assert_eq!(declared_size, new_moov.len(), "moov size 应与实际字节一致");

        // 验证新 moov 包含两个 trak
        let new_moov_payload = &new_moov[8..];
        let trak_boxes = find_child_boxes(new_moov_payload, b"trak").unwrap();
        assert_eq!(trak_boxes.len(), 2, "新 moov 应包含 2 个 trak");
    }

    /// 完整合并流程的端到端测试：构造两个最小 fMP4，合并后验证结构。
    #[test]
    fn merge_minimal_fmp4_files_end_to_end() {
        let video_data = build_minimal_fmp4(1);
        let audio_data = build_minimal_fmp4(1);

        // 写入临时文件
        let temp_dir = std::env::temp_dir();
        let video_path = temp_dir.join("test_merge_video.mp4");
        let audio_path = temp_dir.join("test_merge_audio.mp4");
        let output_path = temp_dir.join("test_merge_output.mp4");

        std::fs::write(&video_path, &video_data).unwrap();
        std::fs::write(&audio_path, &audio_data).unwrap();

        // 运行合并（使用 tokio runtime）
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            merge_fragmented_mp4(&video_path, &audio_path, &output_path)
                .await
                .expect("合并应成功");
        });

        // 验证输出文件
        let output_data = std::fs::read(&output_path).unwrap();
        let output_boxes = parse_top_level_boxes(&output_data).expect("应解析输出");

        // 输出应包含：ftyp, moov, styp, moof, mdat（视频）, styp, moof, mdat（音频）
        assert!(output_boxes.iter().any(|b| &b.box_type == b"ftyp"));
        assert!(output_boxes.iter().any(|b| &b.box_type == b"moov"));

        // 验证 moov 包含两个 trak
        let moov = output_boxes
            .iter()
            .find(|b| &b.box_type == b"moov")
            .unwrap();
        let moov_bytes = &output_data[moov.offset..moov.offset + moov.size];
        let moov_payload = &moov_bytes[8..];
        let trak_boxes = find_child_boxes(moov_payload, b"trak").unwrap();
        assert_eq!(trak_boxes.len(), 2, "输出 moov 应包含 2 个 trak");

        // 验证第一个 trak 的 track_id 是 1（视频）
        let video_trak = trak_boxes[0];
        let video_trak_bytes = &moov_payload[video_trak.offset..video_trak.offset + video_trak.size];
        // trak_bytes 含 8 字节 header，tkhd 在 payload 中
        let video_trak_payload = &video_trak_bytes[8..];
        let video_tkhd = find_child_boxes(video_trak_payload, b"tkhd").unwrap()[0];
        let video_track_id = read_u32_be(video_trak_payload, video_tkhd.offset + 20).unwrap();
        assert_eq!(video_track_id, 1, "视频 track_id 应为 1");

        // 验证第二个 trak 的 track_id 是 2（音频）
        let audio_trak = trak_boxes[1];
        let audio_trak_bytes = &moov_payload[audio_trak.offset..audio_trak.offset + audio_trak.size];
        let audio_trak_payload = &audio_trak_bytes[8..];
        let audio_tkhd = find_child_boxes(audio_trak_payload, b"tkhd").unwrap()[0];
        let audio_track_id = read_u32_be(audio_trak_payload, audio_tkhd.offset + 20).unwrap();
        assert_eq!(audio_track_id, 2, "音频 track_id 应为 2");

        // 清理临时文件
        let _ = std::fs::remove_file(&video_path);
        let _ = std::fs::remove_file(&audio_path);
        let _ = std::fs::remove_file(&output_path);
    }

    /// 真实 Twitter 视频端到端合并测试（手动运行：`cargo test --lib merge_real_twitter -- --ignored`）。
    ///
    /// 此测试需要预先用 yt-dlp 在无 FFmpeg 环境下下载 Twitter 视频，产生分离的
    /// fMP4 文件。测试会在指定目录查找形如 `*.fhls-1570.mp4` 和 `*.fhls-audio-*.mp4`
    /// 的文件，调用 `merge_fragmented_mp4` 合并，并验证输出结构。
    ///
    /// 路径环境变量 `TW_E2E_TEST_DIR` 指向包含分离文件的目录；未设置时跳过。
    #[tokio::test]
    #[ignore = "需要真实 Twitter 分离文件，通过 TW_E2E_TEST_DIR 环境变量启用"]
    async fn merge_real_twitter_split_files() {
        let test_dir = match std::env::var("TW_E2E_TEST_DIR") {
            Ok(dir) => std::path::PathBuf::from(dir),
            Err(_) => {
                eprintln!("跳过：未设置 TW_E2E_TEST_DIR 环境变量");
                return;
            }
        };

        // 在目录中查找分离的视频和音频文件
        let mut video_path: Option<std::path::PathBuf> = None;
        let mut audio_path: Option<std::path::PathBuf> = None;
        for entry in std::fs::read_dir(&test_dir).expect("读取测试目录失败") {
            let entry = entry.expect("读取目录条目失败");
            let name = entry.file_name().to_string_lossy().to_lowercase();
            if name.contains("fhls-1570") && name.ends_with(".mp4") {
                video_path = Some(entry.path());
            } else if name.contains("fhls-audio") && name.ends_with(".mp4") {
                audio_path = Some(entry.path());
            }
        }

        let video_path = video_path.expect("未找到视频文件 *.fhls-1570.mp4");
        let audio_path = audio_path.expect("未找到音频文件 *.fhls-audio-*.mp4");
        let output_path = test_dir.join("merged_output.mp4");

        println!("视频文件: {:?}", video_path);
        println!("音频文件: {:?}", audio_path);
        println!("输出文件: {:?}", output_path);

        // 调用合并函数
        merge_fragmented_mp4(&video_path, &audio_path, &output_path)
            .await
            .expect("合并真实 Twitter 文件应成功");

        // 验证输出文件存在且非空
        let output_data = std::fs::read(&output_path).expect("读取合并输出失败");
        assert!(!output_data.is_empty(), "输出文件不应为空");

        // 验证 ftyp box 存在
        let output_boxes = parse_top_level_boxes(&output_data).expect("应解析输出");
        assert!(
            output_boxes.iter().any(|b| &b.box_type == b"ftyp"),
            "输出应包含 ftyp"
        );
        assert!(
            output_boxes.iter().any(|b| &b.box_type == b"moov"),
            "输出应包含 moov"
        );

        // 验证 moov 包含两个 trak（视频 + 音频）
        let moov = output_boxes
            .iter()
            .find(|b| &b.box_type == b"moov")
            .expect("应找到 moov");
        let moov_bytes = &output_data[moov.offset..moov.offset + moov.size];
        let moov_payload = &moov_bytes[8..];
        let trak_boxes = find_child_boxes(moov_payload, b"trak").unwrap();
        assert_eq!(trak_boxes.len(), 2, "输出 moov 应包含 2 个 trak");

        // 验证视频 track_id=1
        let video_trak = trak_boxes[0];
        let video_trak_bytes =
            &moov_payload[video_trak.offset..video_trak.offset + video_trak.size];
        let video_trak_payload = &video_trak_bytes[8..];
        let video_tkhd = find_child_boxes(video_trak_payload, b"tkhd").unwrap()[0];
        let video_track_id = read_u32_be(video_trak_payload, video_tkhd.offset + 20).unwrap();
        assert_eq!(video_track_id, 1, "视频 track_id 应为 1");

        // 验证音频 track_id=2
        let audio_trak = trak_boxes[1];
        let audio_trak_bytes =
            &moov_payload[audio_trak.offset..audio_trak.offset + audio_trak.size];
        let audio_trak_payload = &audio_trak_bytes[8..];
        let audio_tkhd = find_child_boxes(audio_trak_payload, b"tkhd").unwrap()[0];
        let audio_track_id = read_u32_be(audio_trak_payload, audio_tkhd.offset + 20).unwrap();
        assert_eq!(audio_track_id, 2, "音频 track_id 应为 2");

        // 验证输出文件大小合理（应大于视频和音频文件大小之和的 99%，因为可能有 box header 开销）
        let video_size = std::fs::metadata(&video_path).unwrap().len();
        let audio_size = std::fs::metadata(&audio_path).unwrap().len();
        let output_size = output_data.len() as u64;
        let expected_min = (video_size + audio_size) * 99 / 100;
        assert!(
            output_size >= expected_min,
            "输出文件大小 {} 应至少为输入之和的 99% ({})",
            output_size,
            expected_min
        );

        println!("合并成功！");
        println!("  视频大小: {} 字节", video_size);
        println!("  音频大小: {} 字节", audio_size);
        println!("  输出大小: {} 字节", output_size);
        println!("  顶级 box 数量: {}", output_boxes.len());
        for b in &output_boxes {
            let t = std::str::from_utf8(&b.box_type).unwrap_or("????");
            println!("    - {} (offset={}, size={})", t, b.offset, b.size);
        }

        // 清理输出文件（保留原始分离文件供再次测试）
        let _ = std::fs::remove_file(&output_path);
    }
}
