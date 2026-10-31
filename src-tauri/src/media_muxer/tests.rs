use super::{boxes::*, *};
use sha2::{Digest, Sha256};

pub(super) fn single_track(kind: TrackKind, id: u32, media: &[u8], absolute: bool) -> Vec<u8> {
    let mut tkhd = vec![0; 84];
    put_u32(&mut tkhd, 12, id).unwrap();
    let mut hdlr = vec![0; 24];
    hdlr[8..12].copy_from_slice(if kind == TrackKind::Video {
        b"vide"
    } else {
        b"soun"
    });
    let mut sample_payload = vec![0; if kind == TrackKind::Video { 78 } else { 28 }];
    sample_payload[6..8].copy_from_slice(&[0, 1]);
    let sample = boxed(
        if kind == TrackKind::Video {
            b"avc1"
        } else {
            b"mp4a"
        },
        &sample_payload,
    )
    .unwrap();
    let mut stsd = vec![0; 8];
    put_u32(&mut stsd, 4, 1).unwrap();
    stsd.extend(sample);
    let stbl = boxed(b"stbl", &boxed(b"stsd", &stsd).unwrap()).unwrap();
    let mut mdia = boxed(b"hdlr", &hdlr).unwrap();
    mdia.extend(boxed(b"minf", &stbl).unwrap());
    let mut trak = boxed(b"tkhd", &tkhd).unwrap();
    trak.extend(boxed(b"mdia", &mdia).unwrap());
    let mut trex = vec![0; 24];
    put_u32(&mut trex, 4, id).unwrap();
    put_u32(&mut trex, 8, 1).unwrap();
    let mut moov = boxed(b"mvhd", &vec![0; 100]).unwrap();
    moov.extend(boxed(b"trak", &trak).unwrap());
    moov.extend(boxed(b"mvex", &boxed(b"trex", &trex).unwrap()).unwrap());
    let mut file = boxed(b"ftyp", b"isom\0\0\0\0").unwrap();
    file.extend(boxed(b"moov", &moov).unwrap());
    file.extend(boxed(b"styp", b"msdh\0\0\0\0").unwrap());
    let old_moof = file.len() as u64;
    let mut tfhd = vec![0; if absolute { 16 } else { 8 }];
    put_u32(&mut tfhd, 0, if absolute { 1 } else { 0x20000 }).unwrap();
    put_u32(&mut tfhd, 4, id).unwrap();
    let mut trun = vec![0; 16];
    put_u32(&mut trun, 0, 0x201).unwrap();
    put_u32(&mut trun, 4, 1).unwrap();
    put_u32(&mut trun, 12, media.len() as u32).unwrap();
    let build = |tfhd: &[u8], trun: &[u8]| {
        let mut traf = boxed(b"tfhd", tfhd).unwrap();
        traf.extend(boxed(b"trun", trun).unwrap());
        let mut body = boxed(b"mfhd", &[0; 8]).unwrap();
        body.extend(boxed(b"traf", &traf).unwrap());
        boxed(b"moof", &body).unwrap()
    };
    let size = build(&tfhd, &trun).len();
    if absolute {
        put_u64(&mut tfhd, 8, old_moof + size as u64 + 8).unwrap();
        put_u32(&mut trun, 8, 0).unwrap();
    } else {
        put_u32(&mut trun, 8, (size + 8) as u32).unwrap();
    }
    file.extend(build(&tfhd, &trun));
    file.extend(((media.len() + 8) as u32).to_be_bytes());
    file.extend(b"mdat");
    file.extend(media);
    file
}

#[tokio::test]
async fn streaming_merge_preserves_media_and_patches_ids_offsets_and_single_mvex() {
    for absolute in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let video = dir.path().join("misleading-audio.mp4");
        let audio = dir.path().join("larger-video.mp4");
        let out = dir.path().join("out.mp4");
        let v = single_track(TrackKind::Video, 7, b"VIDEO", absolute);
        let a = single_track(TrackKind::Audio, 42, b"AUDIO-BYTES", absolute);
        tokio::fs::write(&video, &v).await.unwrap();
        tokio::fs::write(&audio, &a).await.unwrap();
        assert_eq!(
            probe_fragmented_mp4_track_kind(&video).await.unwrap(),
            TrackKind::Video
        );
        assert_eq!(
            probe_fragmented_mp4_track_kind(&audio).await.unwrap(),
            TrackKind::Audio
        );
        merge_fragmented_mp4(&video, &audio, &out).await.unwrap();
        validate_fragmented_mp4_with_cancel(&out, &CancellationToken::new())
            .await
            .unwrap();
        assert!(
            probe_fragmented_mp4_track_kind(&out).await.is_err(),
            "single-track recognition stays strict"
        );
        let bytes = tokio::fs::read(out).await.unwrap();
        let top = children(&bytes).unwrap();
        let moov = payload(&bytes, only(&bytes, b"moov").unwrap());
        assert_eq!(
            children(moov)
                .unwrap()
                .iter()
                .filter(|item| item.kind == *b"mvex")
                .count(),
            1
        );
        assert_eq!(
            children(payload(moov, only(moov, b"mvex").unwrap()))
                .unwrap()
                .len(),
            2
        );
        let mdats: Vec<_> = top
            .iter()
            .filter(|item| item.kind == *b"mdat")
            .copied()
            .collect();
        assert_eq!(
            Sha256::digest(payload(&bytes, mdats[0])),
            Sha256::digest(b"VIDEO")
        );
        assert_eq!(
            Sha256::digest(payload(&bytes, mdats[1])),
            Sha256::digest(b"AUDIO-BYTES")
        );
        for (index, item) in top.iter().filter(|item| item.kind == *b"moof").enumerate() {
            let moof = payload(&bytes, *item);
            let traf = payload(moof, only(moof, b"traf").unwrap());
            let tfhd = payload(traf, only(traf, b"tfhd").unwrap());
            assert_eq!(u32_at(tfhd, 4).unwrap(), index as u32 + 1);
            assert_eq!(
                u32_at(payload(moof, only(moof, b"mfhd").unwrap()), 4).unwrap(),
                index as u32 + 1
            );
            if absolute {
                assert_eq!(
                    u64_at(tfhd, 8).unwrap(),
                    (mdats[index].offset + mdats[index].header) as u64
                );
            }
        }
        assert_eq!(tokio::fs::read(video).await.unwrap(), v);
        assert_eq!(tokio::fs::read(audio).await.unwrap(), a);
    }
}

#[tokio::test]
async fn malformed_ts_progressive_drm_and_invalid_sample_offsets_never_publish() {
    let valid = single_track(TrackKind::Video, 1, b"VIDEO", false);
    let mut malformed = Vec::new();
    malformed.push(vec![0x47; 188]);
    malformed.push(valid[..20].to_vec());
    let mut bad = valid.clone();
    bad[..4].copy_from_slice(&4u32.to_be_bytes());
    malformed.push(bad);
    let mut bad = valid.clone();
    let at = bad.windows(4).position(|b| b == b"avc1").unwrap();
    bad[at..at + 4].copy_from_slice(b"encv");
    malformed.push(bad);
    let mut bad = valid.clone();
    let at = bad.windows(4).position(|b| b == b"trun").unwrap();
    put_u32(&mut bad, at + 12, 0x7fffffff).unwrap();
    malformed.push(bad);
    let mut bad = valid.clone();
    let at = bad.windows(4).position(|b| b == b"moof").unwrap();
    bad.truncate(at - 4);
    malformed.push(bad);
    for video_data in malformed {
        let dir = tempfile::tempdir().unwrap();
        let video = dir.path().join("v");
        let audio = dir.path().join("a");
        let out = dir.path().join("out");
        tokio::fs::write(&video, &video_data).await.unwrap();
        tokio::fs::write(&audio, single_track(TrackKind::Audio, 1, b"A", false))
            .await
            .unwrap();
        assert!(merge_fragmented_mp4(&video, &audio, &out).await.is_err());
        assert!(!out.exists());
        assert_eq!(tokio::fs::read(video).await.unwrap(), video_data);
    }
}

#[tokio::test]
async fn zero_length_mdat_is_normalized_before_appending_another_track() {
    let dir = tempfile::tempdir().unwrap();
    let video = dir.path().join("v");
    let audio = dir.path().join("a");
    let out = dir.path().join("out");
    let mut v = single_track(TrackKind::Video, 1, b"V", false);
    let at = v.windows(4).position(|b| b == b"mdat").unwrap();
    v[at - 4..at].fill(0);
    tokio::fs::write(&video, v).await.unwrap();
    tokio::fs::write(&audio, single_track(TrackKind::Audio, 1, b"A", false))
        .await
        .unwrap();
    merge_fragmented_mp4(&video, &audio, &out).await.unwrap();
    assert_eq!(
        children(&tokio::fs::read(out).await.unwrap())
            .unwrap()
            .iter()
            .filter(|item| item.kind == *b"mdat")
            .count(),
        2
    );
}

#[tokio::test]
async fn existing_target_and_cancel_preserve_all_inputs() {
    let dir = tempfile::tempdir().unwrap();
    let video = dir.path().join("v");
    let audio = dir.path().join("a");
    let out = dir.path().join("out");
    tokio::fs::write(&video, single_track(TrackKind::Video, 1, b"V", false))
        .await
        .unwrap();
    tokio::fs::write(&audio, single_track(TrackKind::Audio, 1, b"A", false))
        .await
        .unwrap();
    tokio::fs::write(&out, b"SENTINEL").await.unwrap();
    assert!(merge_fragmented_mp4(&video, &audio, &out).await.is_err());
    assert_eq!(tokio::fs::read(&out).await.unwrap(), b"SENTINEL");
    let cancel = CancellationToken::new();
    cancel.cancel();
    let error =
        merge_fragmented_mp4_with_cancel(&video, &audio, &dir.path().join("cancelled"), &cancel)
            .await
            .unwrap_err();
    assert!(error.contains("取消"));
    assert!(video.exists() && audio.exists());
    assert!(!dir.path().join("cancelled").exists());
}

#[tokio::test]
async fn metadata_limit_is_checked_before_allocating_large_box() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("oversized");
    let mut data = boxed(b"ftyp", b"isom\0\0\0\0").unwrap();
    let size = MAX_METADATA + 1;
    data.extend_from_slice(&(size as u32).to_be_bytes());
    data.extend_from_slice(b"moov");
    data.resize(16 + size as usize, 0);
    data.extend_from_slice(&boxed(b"moof", &[]).unwrap());
    tokio::fs::write(&path, data).await.unwrap();
    assert!(probe_fragmented_mp4_track_kind(&path)
        .await
        .unwrap_err()
        .contains("内存上限"));
}

#[tokio::test]
async fn extended_size_moov_and_mdat_are_processed_without_assuming_eight_byte_headers() {
    let dir = tempfile::tempdir().unwrap();
    let video = dir.path().join("v");
    let audio = dir.path().join("a");
    let out = dir.path().join("out");
    let mut value = single_track(TrackKind::Video, 77, b"EXTENDED", false);
    let moov = only(&value, b"moov").unwrap();
    let mut header = Vec::from(1u32.to_be_bytes());
    header.extend(b"moov");
    header.extend(((moov.size + 8) as u64).to_be_bytes());
    value.splice(moov.offset..moov.offset + 8, header);
    let mdat = only(&value, b"mdat").unwrap();
    let mut header = Vec::from(1u32.to_be_bytes());
    header.extend(b"mdat");
    header.extend(((mdat.size + 8) as u64).to_be_bytes());
    value.splice(mdat.offset..mdat.offset + 8, header);
    let trun = value.windows(4).position(|part| part == b"trun").unwrap();
    let old = u32_at(&value, trun + 12).unwrap();
    put_u32(&mut value, trun + 12, old + 8).unwrap();
    tokio::fs::write(&video, value).await.unwrap();
    tokio::fs::write(&audio, single_track(TrackKind::Audio, 8, b"A", false))
        .await
        .unwrap();
    merge_fragmented_mp4(&video, &audio, &out).await.unwrap();
    let bytes = tokio::fs::read(out).await.unwrap();
    let top = children(&bytes).unwrap();
    let media = top.iter().find(|item| item.kind == *b"mdat").unwrap();
    assert_eq!(media.header, 16);
    assert_eq!(payload(&bytes, *media), b"EXTENDED");
}

#[tokio::test]
async fn cancellation_during_mdat_copy_stops_before_atomic_publication() {
    let dir = tempfile::tempdir().unwrap();
    let video = dir.path().join("v");
    let audio = dir.path().join("a");
    let out = dir.path().join("out");
    let media = vec![0x5a; 16 * 1024 * 1024];
    let data = single_track(TrackKind::Video, 1, &media, false);
    let length = data.len();
    tokio::fs::write(&video, data).await.unwrap();
    tokio::fs::write(&audio, single_track(TrackKind::Audio, 1, b"A", false))
        .await
        .unwrap();
    let cancel = CancellationToken::new();
    let stopped = cancel.clone();
    let parent = dir.path().to_owned();
    let watcher = tokio::spawn(async move {
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                let mut entries = tokio::fs::read_dir(&parent).await.unwrap();
                while let Some(entry) = entries.next_entry().await.unwrap() {
                    if entry.file_name().to_string_lossy().contains(".merging-")
                        && entry.metadata().await.unwrap().len() >= 512 * 1024
                    {
                        stopped.cancel();
                        return;
                    }
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    });
    let result = merge_fragmented_mp4_with_cancel(&video, &audio, &out, &cancel).await;
    watcher.await.unwrap();
    assert!(result.unwrap_err().contains("取消"));
    assert!(!out.exists());
    assert_eq!(
        tokio::fs::metadata(video).await.unwrap().len(),
        length as u64
    );
    assert!(audio.exists());
}

#[tokio::test]
async fn dual_track_validation_requires_complete_fragments_and_keeps_single_probe_strict() {
    let dir = tempfile::tempdir().unwrap();
    let video = dir.path().join("v");
    let audio = dir.path().join("a");
    let out = dir.path().join("out");
    tokio::fs::write(&video, single_track(TrackKind::Video, 7, b"VIDEO", false))
        .await
        .unwrap();
    tokio::fs::write(&audio, single_track(TrackKind::Audio, 19, b"AUDIO", false))
        .await
        .unwrap();
    merge_fragmented_mp4(&video, &audio, &out).await.unwrap();
    let original = tokio::fs::read(&out).await.unwrap();
    let top = children(&original).unwrap();
    let moofs: Vec<_> = top
        .iter()
        .filter(|item| item.kind == *b"moof")
        .copied()
        .collect();
    let mut trafs = Vec::new();
    let mut mfhd = Vec::new();
    for item in &moofs {
        let moof = payload(&original, *item);
        let traf = only(moof, b"traf").unwrap();
        trafs.push(moof[traf.offset..traf.offset + traf.size].to_vec());
        let seq = only(moof, b"mfhd").unwrap();
        mfhd = moof[seq.offset..seq.offset + seq.size].to_vec();
    }
    let moof_size = 8 + mfhd.len() + trafs.iter().map(Vec::len).sum::<usize>();
    for (index, traf) in trafs.iter_mut().enumerate() {
        let trun = only(&traf[8..], b"trun").unwrap();
        put_u32(
            traf,
            8 + trun.offset + trun.header + 8,
            (moof_size + 8 + index * 5) as u32,
        )
        .unwrap();
    }
    let mut body = mfhd;
    for traf in trafs {
        body.extend(traf);
    }
    let moof = boxed(b"moof", &body).unwrap();
    let mut both = original[..moofs[0].offset].to_vec();
    both.extend(moof);
    both.extend(boxed(b"mdat", b"VIDEOAUDIO").unwrap());
    tokio::fs::write(&out, &both).await.unwrap();
    validate_fragmented_mp4_with_cancel(&out, &CancellationToken::new())
        .await
        .unwrap();
    assert!(probe_fragmented_mp4_track_kind(&out).await.is_err());
    assert_eq!(tokio::fs::read(&out).await.unwrap(), both);
    both.pop();
    tokio::fs::write(&out, &both).await.unwrap();
    assert!(
        validate_fragmented_mp4_with_cancel(&out, &CancellationToken::new())
            .await
            .is_err()
    );
}
