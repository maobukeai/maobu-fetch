use super::legacy_boxes::*;
use super::*;

/// 构造一个最小化的 fMP4 字节用于测试 box 解析。
///
/// 结构：ftyp + moov(mvhd + trak(tkhd) + mvex(trex)) + styp + moof(traf(tfhd)) + mdat
fn build_minimal_fmp4(track_id: u32) -> Vec<u8> {
    super::tests::single_track(TrackKind::Video, track_id, b"PAYLOAD", false)
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
    let moov_payload = moov_bytes[8..].to_vec();

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
    let video_moov = video_boxes.iter().find(|b| &b.box_type == b"moov").unwrap();
    let video_moov_bytes = &video_data[video_moov.offset..video_moov.offset + video_moov.size];

    let audio_data = super::tests::single_track(TrackKind::Audio, 1, b"AUDIO", false);
    let audio_boxes = parse_top_level_boxes(&audio_data).unwrap();
    let audio_moov = audio_boxes.iter().find(|b| &b.box_type == b"moov").unwrap();
    let audio_moov_bytes = &audio_data[audio_moov.offset..audio_moov.offset + audio_moov.size];
    let audio_moov_payload = &audio_moov_bytes[8..];
    let audio_trak_bytes = extract_and_patch_audio_trak(audio_moov_payload, 2).unwrap();

    // 提取音频 mvex
    let audio_mvex_bytes = {
        let mvex_boxes = find_child_boxes(audio_moov_payload, b"mvex").unwrap();
        let mvex = mvex_boxes[0];
        let mut bytes = audio_moov_payload[mvex.offset..mvex.offset + mvex.size].to_vec();
        let payload = &mut bytes[8..];
        patch_trex_track_id(payload, 2).unwrap();
        bytes
    };

    let new_moov =
        build_merged_moov_with_extras(video_moov_bytes, &audio_trak_bytes, Some(&audio_mvex_bytes))
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
    let audio_data = super::tests::single_track(TrackKind::Audio, 1, b"AUDIO", false);

    // 写入临时文件
    let temp_root = tempfile::tempdir().unwrap();
    let temp_dir = temp_root.path();
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
    let video_trak_bytes = &moov_payload[video_trak.offset..video_trak.offset + video_trak.size];
    let video_trak_payload = &video_trak_bytes[8..];
    let video_tkhd = find_child_boxes(video_trak_payload, b"tkhd").unwrap()[0];
    let video_track_id = read_u32_be(video_trak_payload, video_tkhd.offset + 20).unwrap();
    assert_eq!(video_track_id, 1, "视频 track_id 应为 1");

    // 验证音频 track_id=2
    let audio_trak = trak_boxes[1];
    let audio_trak_bytes = &moov_payload[audio_trak.offset..audio_trak.offset + audio_trak.size];
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
