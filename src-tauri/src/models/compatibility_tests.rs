use super::*;

#[test]
fn old_bt_metadata_keeps_seeding_disabled_and_paths_unknown() {
    let meta: BtTaskMeta = serde_json::from_str("{}").unwrap();
    assert!(!meta.seeding_requested);
    assert!(!meta.output_owned);
    assert!(meta.output_folder.is_none());
    assert!(meta.owned_files.is_empty());
    assert_eq!(meta.uploaded_bytes, 0);
    let runtime: BtRuntimeStatus = serde_json::from_value(serde_json::json!({
        "num_seeds":0,"num_peers":2,"upload_speed":0,"fetching_metadata":false
    })).unwrap();
    assert!(!runtime.seeds_known);
    assert!(!runtime.seeding);
}

#[test]
fn old_media_results_do_not_claim_ffmpeg_or_an_invented_bitrate() {
    let result: MediaProbeResult = serde_json::from_value(serde_json::json!({
        "title":"旧版媒体","formats":[{"id":"direct","label":"360p",
        "has_video":true,"has_audio":true}],"subtitles":[],"drm":false
    })).unwrap();
    assert!(!result.ffmpeg_available);
    assert!(result.formats[0].bitrate_kbps.is_none());
    assert!(!result.formats[0].original_file);
    assert!(!result.formats[0].audio_unknown);
    assert!(result.episodes.is_empty());
}
