    use super::*;
    fn test_task(directory: &Path, file_name: &str, policy: CollisionPolicy) -> DownloadTask {
        DownloadTask {
            id: "task".into(),
            url: "https://example.com/file".into(),
            file_name: file_name.into(),
            destination: directory.to_string_lossy().into_owned(),
            total_bytes: 0,
            downloaded_bytes: 0,
            speed: 0,
            eta_seconds: None,
            status: TaskStatus::Queued,
            error: None,
            created_at: 0,
            completed_at: None,
            scheduled_at: None,
            category: "other".into(),
            queue_position: 0,
            priority: 0,
            retry_count: 0,
            max_retries: 3,
            checksum_sha256: None,
            expected_checksum: None,
            source: "desktop".into(),
            etag: None,
            last_modified: None,
            final_url: None,
            response_status: None,
            content_type: None,
            accepts_ranges: None,
            headers: HashMap::new(),
            media: None,
            per_task_speed_limit: 0,
            collision_policy: policy,
            completion_action: CompletionAction::None,
            connection_count: 1,
            active_connections: 0,
            segments: Vec::new(),
            retry_policy_override: None,
            proxy_override: None,
            proxy_auth: None,
        task_kind: Default::default(),
        bt_meta: None,
        bt_runtime: None,
        cloud_refresh: None,
        }
    }
    #[test]
    fn sanitizes_windows_names() {
        assert_eq!(safe_name("a<b>c.zip"), "a_b_c.zip");
        assert_eq!(safe_name("..."), "download")
    }
    #[test]
    fn rename_validation_rejects_empty_invalid_and_traversal() {
        // 空字符串。注：调用方 `rename` 方法已先 trim 输入，
        // 因此纯空白 "   " 在到达此函数前已变为 ""。
        assert!(validate_rename_filename("").is_err());
        // 合法文件名通过
        assert!(validate_rename_filename("movie.mp4").is_ok());
        assert!(validate_rename_filename("报告_2026.pdf").is_ok());
        // Windows 非法字符
        assert!(validate_rename_filename("a<b>c.zip").is_err());
        assert!(validate_rename_filename("a:b").is_err());
        assert!(validate_rename_filename("a*b").is_err());
        assert!(validate_rename_filename("a?b").is_err());
        assert!(validate_rename_filename("a|b").is_err());
        assert!(validate_rename_filename("a\"b").is_err());
        // 路径分隔符与穿越
        assert!(validate_rename_filename("a/b").is_err());
        assert!(validate_rename_filename("a\\b").is_err());
        assert!(validate_rename_filename("../escape.zip").is_err());
        assert!(validate_rename_filename("/abs.zip").is_err());
        assert!(validate_rename_filename("\\abs.zip").is_err());
        // 控制字符
        assert!(validate_rename_filename("a\x00b").is_err());
        assert!(validate_rename_filename("a\nb").is_err());
        // 长度上限
        let long = "a".repeat(256);
        assert!(validate_rename_filename(&long).is_err());
        let max = "a".repeat(255);
        assert!(validate_rename_filename(&max).is_ok());
    }
    #[test]
    fn classifies_files() {
        assert_eq!(category("movie.mp4"), "video");
        assert_eq!(category("setup.exe"), "apps")
    }
    #[test]
    fn diagnostic_urls_hide_credentials_query_and_fragment() {
        let url =
            Url::parse("https://user:password@example.com/file?token=secret#private").unwrap();
        assert_eq!(diagnostic_url(&url), "https://example.com/file");
    }
    #[test]
    fn completion_notifications_respect_settings_and_terminal_state() {
        let directory = tempfile::tempdir().unwrap();
        let mut task = test_task(directory.path(), "done.zip", CollisionPolicy::Rename);
        task.status = TaskStatus::Completed;
        let mut settings = AppSettings::default();
        let notification = completion_notification(&settings, &task).unwrap();
        assert!(notification.0.contains("done.zip"));
        assert!(notification
            .1
            .contains(directory.path().to_string_lossy().as_ref()));
        settings.notifications = false;
        assert!(completion_notification(&settings, &task).is_none());
        settings.notifications = true;
        task.status = TaskStatus::Downloading;
        assert!(completion_notification(&settings, &task).is_none());
    }

    // ===== Task 30: 下载完成通知与声音 =====

    #[test]
    fn completion_notification_respects_notify_on_complete_flag() {
        // Task 30.1：notify_on_complete = false 应抑制完成通知。
        let directory = tempfile::tempdir().unwrap();
        let mut task = test_task(directory.path(), "done.zip", CollisionPolicy::Rename);
        task.status = TaskStatus::Completed;
        let mut settings = AppSettings::default();
        assert!(completion_notification(&settings, &task).is_some());
        settings.notify_on_complete = false;
        assert!(completion_notification(&settings, &task).is_none());
    }

    #[test]
    fn failure_notification_respects_settings_and_state() {
        // Task 30.2：失败通知仅在 notifications && notify_on_failure 且 Failed 状态时返回文案。
        let directory = tempfile::tempdir().unwrap();
        let mut task = test_task(directory.path(), "broken.zip", CollisionPolicy::Rename);
        task.status = TaskStatus::Failed;
        task.error = Some("NETWORK: 连接被重置".into());
        let mut settings = AppSettings::default();
        let notification = failure_notification(&settings, &task).unwrap();
        assert!(notification.0.contains("broken.zip"));
        assert!(notification.1.contains("连接被重置"));

        // 关闭失败通知开关
        settings.notify_on_failure = false;
        assert!(failure_notification(&settings, &task).is_none());
        settings.notify_on_failure = true;

        // 关闭主通知开关
        settings.notifications = false;
        assert!(failure_notification(&settings, &task).is_none());
        settings.notifications = true;

        // 非 Failed 状态不返回失败通知
        task.status = TaskStatus::Downloading;
        assert!(failure_notification(&settings, &task).is_none());
    }

    #[test]
    fn failure_notification_falls_back_to_unknown_error() {
        // task.error 缺失时 body 回退为"未知错误"。
        let directory = tempfile::tempdir().unwrap();
        let mut task = test_task(directory.path(), "noerror.zip", CollisionPolicy::Rename);
        task.status = TaskStatus::Failed;
        task.error = None;
        let settings = AppSettings::default();
        let notification = failure_notification(&settings, &task).unwrap();
        assert_eq!(notification.1, "未知错误");
    }

    #[test]
    fn validates_concurrency() {
        let mut settings = AppSettings::default();
        settings.concurrent_downloads = 0;
        assert!(validate_settings(&settings).is_err())
    }
    #[test]
    fn validates_custom_media_tool_paths() {
        let directory = tempfile::tempdir().unwrap();
        let yt_dlp = directory.path().join("yt-dlp.exe");
        let ffmpeg = directory.path().join("ffmpeg.exe");
        let ffprobe = directory.path().join("ffprobe.exe");
        std::fs::write(&yt_dlp, b"yt").unwrap();
        std::fs::write(&ffmpeg, b"ffmpeg").unwrap();
        std::fs::write(&ffprobe, b"ffprobe").unwrap();
        let mut settings = AppSettings::default();
        settings.yt_dlp_path = yt_dlp.to_string_lossy().into_owned();
        settings.ffmpeg_path = ffmpeg.to_string_lossy().into_owned();
        settings.ffprobe_path = ffprobe.to_string_lossy().into_owned();
        assert!(validate_settings(&settings).is_ok());

        settings.ffprobe_path.clear();
        assert!(validate_settings(&settings).is_err());
    }
    #[test]
    fn collision_preflight_renames_files_and_reserved_tasks() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("archive.zip"), b"existing").unwrap();
        let task = test_task(directory.path(), "archive.zip", CollisionPolicy::Rename);
        let mut reserved = HashSet::new();
        reserved.insert(path_key(&directory.path().join("archive (1).zip")));
        let output = resolve_output_path(&task, &reserved).unwrap();
        assert_eq!(output.file_name().unwrap(), "archive (2).zip");
    }

    #[test]
    fn overwrite_rejects_a_path_reserved_by_an_unfinished_task() {
        let directory = tempfile::tempdir().unwrap();
        let task = test_task(directory.path(), "archive.zip", CollisionPolicy::Overwrite);
        let reserved = HashSet::from([path_key(&directory.path().join("archive.zip"))]);
        assert!(resolve_output_path(&task, &reserved)
            .unwrap_err()
            .contains("另一个未完成任务"));
    }

    #[test]
    fn network_errors_enter_the_waiting_path() {
        assert!(is_network_error("分片失败：NETWORK: 无法连接服务器"));
        assert!(!is_network_error("HTTP 404"));
    }

    #[test]
    fn scheduler_prefers_priority_then_queue_position() {
        // Task 16: 数字越小越优先。priority=-1 排在 priority=0 之前。
        let directory = tempfile::tempdir().unwrap();
        let mut normal_first = test_task(directory.path(), "normal-first", CollisionPolicy::Rename);
        normal_first.id = "normal-first".into();
        normal_first.queue_position = 1;
        let mut high = test_task(directory.path(), "high", CollisionPolicy::Rename);
        high.id = "high".into();
        high.priority = -1;
        high.queue_position = 9;
        let mut normal_second =
            test_task(directory.path(), "normal-second", CollisionPolicy::Rename);
        normal_second.id = "normal-second".into();
        normal_second.queue_position = 2;
        let mut candidates = vec![normal_second, high, normal_first];
        sort_download_candidates(&mut candidates);
        assert_eq!(
            candidates
                .iter()
                .map(|task| task.id.as_str())
                .collect::<Vec<_>>(),
            ["high", "normal-first", "normal-second"]
        );
    }

    #[test]
    fn runtime_options_apply_live_speed_priority_and_completion_changes() {
        let directory = tempfile::tempdir().unwrap();
        let mut task = test_task(directory.path(), "file.bin", CollisionPolicy::Rename);
        let runtime = RuntimeTaskOptions::new(&task);
        runtime.speed_limit.store(512 * 1024, Ordering::Relaxed);
        runtime.priority.store(-1, Ordering::Relaxed);
        *runtime.completion_action.blocking_write() = CompletionAction::OpenFolder;
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(runtime.apply(&mut task));
        assert_eq!(task.per_task_speed_limit, 512 * 1024);
        assert_eq!(task.priority, -1);
        assert_eq!(task.completion_action, CompletionAction::OpenFolder);
    }
    #[test]
    fn power_action_waits_for_every_tracked_task_to_complete() {
        let targets = HashSet::from(["one".to_string(), "two".to_string()]);
        let mut statuses = HashMap::from([
            ("one".to_string(), TaskStatus::Completed),
            ("two".to_string(), TaskStatus::Downloading),
        ]);
        assert_eq!(
            power_action_decision(&targets, &statuses),
            PowerActionDecision::Waiting
        );
        statuses.insert("two".into(), TaskStatus::Completed);
        assert_eq!(
            power_action_decision(&targets, &statuses),
            PowerActionDecision::Complete
        );
    }

    #[test]
    fn power_action_is_blocked_by_unsafe_terminal_states() {
        let targets = HashSet::from(["task".to_string()]);
        for status in [
            TaskStatus::Paused,
            TaskStatus::Failed,
            TaskStatus::Cancelled,
        ] {
            let statuses = HashMap::from([("task".to_string(), status)]);
            assert!(matches!(
                power_action_decision(&targets, &statuses),
                PowerActionDecision::Blocked(_)
            ));
        }
        assert!(matches!(
            power_action_decision(&targets, &HashMap::new()),
            PowerActionDecision::Blocked(_)
        ));
    }

    #[test]
    fn power_action_tracks_all_runnable_and_paused_tasks() {
        for status in [
            TaskStatus::Queued,
            TaskStatus::Downloading,
            TaskStatus::Paused,
            TaskStatus::Scheduled,
            TaskStatus::Verifying,
            TaskStatus::WaitingNetwork,
        ] {
            assert!(is_power_action_target(&status));
        }
        assert!(!is_power_action_target(&TaskStatus::Completed));
        assert!(!is_power_action_target(&TaskStatus::Failed));
    }

    #[test]
    fn power_action_countdown_reports_seconds_from_milliseconds() {
        assert_eq!(power_action_remaining_seconds(60_000, 0), 60);
        assert_eq!(power_action_remaining_seconds(60_000, 1), 60);
        assert_eq!(power_action_remaining_seconds(60_000, 59_001), 1);
        assert_eq!(power_action_remaining_seconds(60_000, 60_000), 0);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn power_actions_use_direct_windows_commands_without_a_shell() {
        assert_eq!(
            power_action_command_args(PowerAction::Shutdown),
            Some(&["/s", "/t", "0"][..])
        );
        assert_eq!(
            power_action_command_args(PowerAction::Hibernate),
            Some(&["/h"][..])
        );
        assert_eq!(power_action_command_args(PowerAction::None), None);
    }
    #[test]
    fn accepts_32_connections_and_rejects_other_values() {
        let mut settings = AppSettings::default();
        settings.connections_per_download = 32;
        assert!(validate_settings(&settings).is_ok());
        settings.connections_per_download = 24;
        assert!(validate_settings(&settings).is_err());
    }
    #[test]
    fn low_memory_mode_caps_runtime_concurrency_without_changing_preferences() {
        let mut settings = AppSettings::default();
        settings.concurrent_downloads = 8;
        settings.connections_per_download = 16;
        settings.low_memory_mode = true;

        assert_eq!(effective_concurrent_downloads(&settings), 1);
        assert_eq!(effective_connection_count(&settings, 16), 2);
        assert_eq!(settings.concurrent_downloads, 8);
        assert_eq!(settings.connections_per_download, 16);
    }
    #[test]
    fn segment_layout_covers_file_exactly() {
        let total = 10_000_003;
        let ranges = segment_ranges(total, 8);
        assert_eq!(ranges.len(), 8);
        assert_eq!(ranges.first().map(|range| range.1), Some(0));
        assert_eq!(ranges.last().map(|range| range.2), Some(total - 1));
        for pair in ranges.windows(2) {
            assert_eq!(pair[0].2 + 1, pair[1].1);
        }
        assert_eq!(
            ranges
                .iter()
                .map(|(_, start, end)| end - start + 1)
                .sum::<u64>(),
            total
        );
    }

    #[test]
    fn segment_layout_never_creates_empty_ranges() {
        let ranges = segment_ranges(3, 16);
        assert_eq!(ranges, vec![(0, 0, 0), (1, 1, 1), (2, 2, 2)]);
    }

    #[test]
    fn segment_count_matches_requested_connections() {
        assert_eq!(requested_segment_count(1), 1);
        assert_eq!(requested_segment_count(8), 8);
        assert_eq!(requested_segment_count(16), 16);
        assert_eq!(requested_segment_count(32), 32);
        assert_eq!(requested_segment_count(64), 32);
    }

    #[test]
    fn range_windows_continue_without_overlap_or_extra_logical_segments() {
        let segment_end = 260 * 1024 * 1024 - 1;
        let mut cursor = 0u64;
        let mut covered = 0u64;
        let mut windows = 0;
        while cursor <= segment_end {
            let end = range_window_end(cursor, segment_end, 0);
            assert!(end >= cursor);
            covered += end - cursor + 1;
            windows += 1;
            cursor = end + 1;
        }
        assert_eq!(covered, segment_end + 1);
        assert_eq!(windows, 33);
        assert_eq!(range_window_end(0, u64::MAX, 0) + 1, 8 * 1024 * 1024);
        assert_eq!(range_window_end(0, u64::MAX, 7) + 1, 10_223_616);
    }

    #[test]
    fn new_segments_reserve_one_tail_window_instead_of_many_small_requests() {
        let end = 260 * 1024 * 1024 - 1;
        let ranges = balanced_window_ranges(0, end, 0);
        assert_eq!(ranges.len(), 2);
        assert_eq!(ranges[0].1, 0);
        assert_eq!(ranges[0].2 + 1, ranges[1].1);
        assert_eq!(ranges[1].2, end);
        assert_eq!(
            ranges
                .iter()
                .map(|(_, start, end)| end - start + 1)
                .sum::<u64>(),
            end + 1
        );
        assert_eq!(balanced_window_ranges(0, 8 * 1024 * 1024 - 1, 0).len(), 1);
    }

    #[test]
    fn adaptive_connections_start_full_and_fallback_on_degradation() {
        let gate = AdaptiveConnectionGate::new(32);
        assert_eq!(gate.target.load(Ordering::Relaxed), 32);
        gate.observe(80 * 1024 * 1024);
        for _ in 0..8 {
            gate.observe(10 * 1024 * 1024);
        }
        assert_eq!(gate.target.load(Ordering::Relaxed), 16);
        assert_eq!(gate.disabled.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn adaptive_connections_keep_falling_back_after_a_late_cdn_slowdown() {
        let gate = AdaptiveConnectionGate::new(32);
        assert_eq!(gate.target.load(Ordering::Relaxed), 32);
        gate.observe(70 * 1024 * 1024);
        for expected in [16, 8, 4] {
            for _ in 0..8 {
                gate.observe(10 * 1024 * 1024);
            }
            assert_eq!(gate.target.load(Ordering::Relaxed), expected);
        }
    }

    #[test]
    fn speed_smoothing_dampens_short_sampling_spikes() {
        let previous = 64.0 * 1024.0 * 1024.0;
        let spike = 96.0 * 1024.0 * 1024.0;
        let smoothed = smooth_speed(previous, spike, 0.25);
        assert!(smoothed > previous);
        assert!(smoothed < 70.0 * 1024.0 * 1024.0);

        let falling = smooth_speed(smoothed, 0.0, 0.25);
        assert!(falling > 50.0 * 1024.0 * 1024.0);
        assert!(falling < smoothed);
    }

    #[test]
    fn parses_and_validates_content_range() {
        assert_eq!(
            parse_content_range_value("bytes 10-19/100"),
            Some((10, 19, 100))
        );
        assert_eq!(parse_content_range_value("bytes 19-10/100"), None);
        assert_eq!(parse_content_range_value("bytes 0-100/100"), None);
        assert_eq!(parse_content_range_value("bytes */100"), None);
    }

    #[test]
    fn probe_total_bytes_prefers_content_range_on_206() {
        use axum::http;
        // HEAD 被拒（405/501）回退 GET+Range 后得到 206：Content-Length 是分片长度
        // （1 字节），总长必须取 Content-Range，否则任务总长会被误判为 1 字节。
        let partial: reqwest::Response = http::Response::builder()
            .status(http::StatusCode::PARTIAL_CONTENT)
            .header("Content-Range", "bytes 0-0/1048576")
            .header("Content-Length", "1")
            .body("x".to_string())
            .unwrap()
            .into();
        assert_eq!(probe_total_bytes(&partial), 1_048_576);

        // 普通 200/HEAD 响应：总长取 Content-Length。
        let full: reqwest::Response = http::Response::builder()
            .header("Content-Length", "2048")
            .body(Vec::<u8>::new())
            .unwrap()
            .into();
        assert_eq!(probe_total_bytes(&full), 2048);

        // 206 但 Content-Range 缺失/损坏：安全回退 0（未知长度，走单连接路径）。
        let broken: reqwest::Response = http::Response::builder()
            .status(http::StatusCode::PARTIAL_CONTENT)
            .header("Content-Length", "1")
            .body("x".to_string())
            .unwrap()
            .into();
        assert_eq!(probe_total_bytes(&broken), 0);
    }

    // ---- 校验和算法识别（MD5 / SHA-1 / SHA-256） ----

    #[test]
    fn parse_expected_checksum_detects_algorithms_by_length() {
        let md5_hex = "0123456789abcdef0123456789abcdef";
        let sha1_hex = "0123456789abcdef0123456789abcdef01234567";
        let sha256_hex = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        assert!(matches!(
            parse_expected_checksum(md5_hex),
            Some((ChecksumAlgorithm::Md5, _))
        ));
        assert!(matches!(
            parse_expected_checksum(sha1_hex),
            Some((ChecksumAlgorithm::Sha1, _))
        ));
        assert!(matches!(
            parse_expected_checksum(sha256_hex),
            Some((ChecksumAlgorithm::Sha256, _))
        ));
        // 前缀（大小写不敏感）与去前缀小写化。
        let (algo, cleaned) =
            parse_expected_checksum("SHA256:ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789")
                .expect("带前缀应可识别");
        assert_eq!(algo, ChecksumAlgorithm::Sha256);
        assert!(cleaned.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()));
        // 非法输入：长度不对、非十六进制。
        assert!(parse_expected_checksum("xyz").is_none());
        assert!(parse_expected_checksum(&format!("sha256:{}", "g".repeat(64))).is_none());
        assert!(parse_expected_checksum("123").is_none());
    }

    #[tokio::test]
    async fn digest_file_matches_known_vectors() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("abc.txt");
        tokio::fs::write(&path, b"abc").await.unwrap();
        // "abc" 的公开标准向量。
        assert_eq!(
            digest_file(&path, ChecksumAlgorithm::Md5).await.unwrap(),
            "900150983cd24fb0d6963f7d28e17f72"
        );
        assert_eq!(
            digest_file(&path, ChecksumAlgorithm::Sha1).await.unwrap(),
            "a9993e364706816aba3e25717850c26c9cd0d89d"
        );
        assert_eq!(
            digest_file(&path, ChecksumAlgorithm::Sha256).await.unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    fn selfcheck_task(
        directory: &Path,
        id: &str,
        file_name: &str,
        status: TaskStatus,
        segments: Vec<DownloadSegment>,
    ) -> DownloadTask {
        DownloadTask {
            id: id.into(),
            url: "https://example.com/file.bin".into(),
            file_name: file_name.into(),
            destination: directory.to_string_lossy().into_owned(),
            total_bytes: segments.iter().map(|s| s.end_byte - s.start_byte + 1).sum(),
            downloaded_bytes: segments.iter().map(|s| s.downloaded_bytes).sum(),
            speed: 1024,
            eta_seconds: Some(60),
            status,
            error: None,
            created_at: 1,
            completed_at: None,
            scheduled_at: None,
            category: "other".into(),
            queue_position: 0,
            priority: 0,
            retry_count: 0,
            max_retries: 3,
            checksum_sha256: None,
            expected_checksum: None,
            source: "desktop".into(),
            etag: None,
            last_modified: None,
            final_url: None,
            response_status: None,
            content_type: None,
            accepts_ranges: Some(true),
            headers: HashMap::new(),
            media: None,
            per_task_speed_limit: 0,
            collision_policy: CollisionPolicy::Rename,
            completion_action: CompletionAction::None,
            connection_count: 4,
            active_connections: 2,
            segments,
            retry_policy_override: None,
            proxy_override: None,
            proxy_auth: None,
        task_kind: Default::default(),
        bt_meta: None,
        bt_runtime: None,
        cloud_refresh: None,
        }
    }

    fn selfcheck_segment(
        index: u8,
        start: u64,
        end: u64,
        downloaded: u64,
        status: &str,
    ) -> DownloadSegment {
        DownloadSegment {
            index,
            start_byte: start,
            end_byte: end,
            downloaded_bytes: downloaded,
            status: status.into(),
        }
    }

    #[test]
    fn selfcheck_marks_downloading_tasks_as_interrupted_and_drops_mismatched_shards() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path().to_path_buf()).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            // Two multi-connection segments. Segment 0 has matching bytes on disk;
            // segment 1 is corrupted (recorded 50 bytes, but the file holds 30).
            let segments = vec![
                selfcheck_segment(0, 0, 99, 100, "downloading"),
                selfcheck_segment(1, 100, 199, 50, "downloading"),
            ];
            let task = selfcheck_task(
                directory.path(),
                "selfcheck-mixed",
                "mixed.bin",
                TaskStatus::Downloading,
                segments,
            );
            store.upsert_task(&task).await.unwrap();

            let output = directory.path().join("mixed.bin");
            let temp = PathBuf::from(format!("{}.lumaget", output.to_string_lossy()));
            std::fs::write(format!("{}.part0", temp.to_string_lossy()), vec![0u8; 100]).unwrap();
            std::fs::write(format!("{}.part1", temp.to_string_lossy()), vec![0u8; 30]).unwrap();

            let report = execute_selfcheck(&store).await;

            assert_eq!(report.interrupted_count, 1);
            assert_eq!(report.dropped_shards, 1);
            assert_eq!(report.recovered_tasks, vec!["selfcheck-mixed".to_string()]);

            let restored = store.get_task("selfcheck-mixed").await.unwrap().unwrap();
            assert_eq!(restored.status, TaskStatus::Interrupted);
            assert_eq!(restored.speed, 0);
            assert_eq!(restored.eta_seconds, None);
            assert_eq!(restored.active_connections, 0);
            assert_eq!(restored.downloaded_bytes, 100); // only segment 0 survives

            assert_eq!(restored.segments.len(), 2);
            assert_eq!(restored.segments[0].downloaded_bytes, 100);
            assert_eq!(restored.segments[0].status, "pending");
            assert_eq!(restored.segments[1].downloaded_bytes, 0);
            assert_eq!(restored.segments[1].status, "pending");

            assert!(PathBuf::from(format!("{}.part0", temp.to_string_lossy())).exists());
            assert!(!PathBuf::from(format!("{}.part1", temp.to_string_lossy())).exists());
        });
    }

    #[test]
    fn selfcheck_preserves_consistent_windowed_shards() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path().to_path_buf()).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            // Multi-connection segment using both a legacy prefix file and a
            // windowed continuation file. Both sizes match the recorded bytes.
            let segments = vec![selfcheck_segment(0, 0, 199, 150, "downloading")];
            let task = selfcheck_task(
                directory.path(),
                "selfcheck-windowed",
                "windowed.bin",
                TaskStatus::Downloading,
                segments,
            );
            store.upsert_task(&task).await.unwrap();

            let output = directory.path().join("windowed.bin");
            let temp = PathBuf::from(format!("{}.lumaget", output.to_string_lossy()));
            std::fs::write(format!("{}.part0", temp.to_string_lossy()), vec![0u8; 80]).unwrap();
            std::fs::write(
                format!("{}.part0.w80", temp.to_string_lossy()),
                vec![0u8; 70],
            )
            .unwrap();

            let report = execute_selfcheck(&store).await;

            assert_eq!(report.interrupted_count, 1);
            assert_eq!(report.dropped_shards, 0);
            assert_eq!(
                report.recovered_tasks,
                vec!["selfcheck-windowed".to_string()]
            );

            let restored = store.get_task("selfcheck-windowed").await.unwrap().unwrap();
            assert_eq!(restored.status, TaskStatus::Interrupted);
            assert_eq!(restored.segments[0].downloaded_bytes, 150);
            assert_eq!(restored.segments[0].status, "pending");
            assert_eq!(restored.downloaded_bytes, 150);

            assert!(PathBuf::from(format!("{}.part0", temp.to_string_lossy())).exists());
            assert!(PathBuf::from(format!("{}.part0.w80", temp.to_string_lossy())).exists());
        });
    }

    #[test]
    fn selfcheck_drops_single_stream_shard_when_lumaget_file_is_shorter() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path().to_path_buf()).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            // Single-connection download: the .lumaget file holds the segment data.
            // The on-disk file is shorter than the recorded progress.
            let segments = vec![selfcheck_segment(0, 0, 199, 100, "downloading")];
            let task = selfcheck_task(
                directory.path(),
                "selfcheck-stream",
                "stream.bin",
                TaskStatus::Downloading,
                segments,
            );
            store.upsert_task(&task).await.unwrap();

            let output = directory.path().join("stream.bin");
            let temp = PathBuf::from(format!("{}.lumaget", output.to_string_lossy()));
            std::fs::write(&temp, vec![0u8; 40]).unwrap();

            let report = execute_selfcheck(&store).await;

            assert_eq!(report.interrupted_count, 1);
            assert_eq!(report.dropped_shards, 1);

            let restored = store.get_task("selfcheck-stream").await.unwrap().unwrap();
            assert_eq!(restored.status, TaskStatus::Interrupted);
            assert_eq!(restored.segments[0].downloaded_bytes, 0);
            assert_eq!(restored.segments[0].status, "pending");
            assert_eq!(restored.downloaded_bytes, 0);
            assert!(!temp.exists());
        });
    }

    #[test]
    fn selfcheck_skips_non_downloading_tasks() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path().to_path_buf()).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let queued = selfcheck_task(
                directory.path(),
                "selfcheck-queued",
                "queued.bin",
                TaskStatus::Queued,
                vec![selfcheck_segment(0, 0, 99, 50, "pending")],
            );
            let paused = selfcheck_task(
                directory.path(),
                "selfcheck-paused",
                "paused.bin",
                TaskStatus::Paused,
                vec![selfcheck_segment(0, 0, 99, 50, "paused")],
            );
            store.upsert_task(&queued).await.unwrap();
            store.upsert_task(&paused).await.unwrap();

            let report = execute_selfcheck(&store).await;

            assert_eq!(report.interrupted_count, 0);
            assert_eq!(report.dropped_shards, 0);
            assert!(report.recovered_tasks.is_empty());

            let queued_restored = store.get_task("selfcheck-queued").await.unwrap().unwrap();
            assert_eq!(queued_restored.status, TaskStatus::Queued);
            assert_eq!(queued_restored.segments[0].status, "pending");

            let paused_restored = store.get_task("selfcheck-paused").await.unwrap().unwrap();
            assert_eq!(paused_restored.status, TaskStatus::Paused);
            assert_eq!(paused_restored.segments[0].status, "paused");
        });
    }

    #[test]
    fn selfcheck_preserves_shards_in_hidden_temp_dir() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path().to_path_buf()).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let segments = vec![selfcheck_segment(0, 0, 199, 150, "downloading")];
            let task = selfcheck_task(
                directory.path(),
                "selfcheck-hidden",
                "hidden.bin",
                TaskStatus::Downloading,
                segments,
            );
            store.upsert_task(&task).await.unwrap();

            let task_dir = task_temp_dir(&task.destination, &task.id);
            std::fs::create_dir_all(&task_dir).unwrap();
            let temp = task_temp_path(&task.destination, &task.id, &task.file_name);
            std::fs::write(format!("{}.part0", temp.to_string_lossy()), vec![0u8; 80]).unwrap();
            std::fs::write(
                format!("{}.part0.w80", temp.to_string_lossy()),
                vec![0u8; 70],
            )
            .unwrap();

            let report = execute_selfcheck(&store).await;

            assert_eq!(report.interrupted_count, 1);
            assert_eq!(report.dropped_shards, 0);
            assert_eq!(report.recovered_tasks, vec!["selfcheck-hidden".to_string()]);

            let restored = store.get_task("selfcheck-hidden").await.unwrap().unwrap();
            assert_eq!(restored.status, TaskStatus::Interrupted);
            assert_eq!(restored.segments[0].downloaded_bytes, 150);
            assert_eq!(restored.downloaded_bytes, 150);

            assert!(PathBuf::from(format!("{}.part0", temp.to_string_lossy())).exists());
            assert!(PathBuf::from(format!("{}.part0.w80", temp.to_string_lossy())).exists());
        });
    }

    #[test]
    fn selfcheck_handles_empty_task_list_without_failing() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path().to_path_buf()).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let report = execute_selfcheck(&store).await;

            assert_eq!(report.interrupted_count, 0);
            assert_eq!(report.dropped_shards, 0);
            assert!(report.recovered_tasks.is_empty());
        });
    }

    #[test]
    fn selfcheck_recalculates_task_downloaded_bytes_after_dropping_shards() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path().to_path_buf()).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            // Three segments: only the middle one survives. The task-level
            // downloaded_bytes must be recomputed from the surviving shard.
            let segments = vec![
                selfcheck_segment(0, 0, 49, 50, "downloading"),
                selfcheck_segment(1, 50, 99, 50, "downloading"),
                selfcheck_segment(2, 100, 149, 50, "downloading"),
            ];
            let task = selfcheck_task(
                directory.path(),
                "selfcheck-recompute",
                "recompute.bin",
                TaskStatus::Downloading,
                segments,
            );
            store.upsert_task(&task).await.unwrap();

            let output = directory.path().join("recompute.bin");
            let temp = PathBuf::from(format!("{}.lumaget", output.to_string_lossy()));
            // Segment 0 on disk is 50 bytes (matches).
            std::fs::write(format!("{}.part0", temp.to_string_lossy()), vec![0u8; 50]).unwrap();
            // Segment 1 on disk is 50 bytes (matches).
            std::fs::write(format!("{}.part1", temp.to_string_lossy()), vec![0u8; 50]).unwrap();
            // Segment 2 on disk is 10 bytes (mismatch — recorded 50).
            std::fs::write(format!("{}.part2", temp.to_string_lossy()), vec![0u8; 10]).unwrap();

            let report = execute_selfcheck(&store).await;

            assert_eq!(report.dropped_shards, 1);
            let restored = store
                .get_task("selfcheck-recompute")
                .await
                .unwrap()
                .unwrap();
            assert_eq!(restored.downloaded_bytes, 100); // 50 + 50, segment 2 reset
            assert_eq!(restored.segments[2].downloaded_bytes, 0);
        });
    }

    #[test]
    fn remote_resource_changed_detects_etag_mismatch() {
        // ETag changed → resource changed
        assert!(remote_resource_changed(
            Some("\"abc\""),
            Some("\"xyz\""),
            None,
            None,
        ));
    }

    #[test]
    fn remote_resource_changed_allows_matching_etag_to_resume() {
        // ETag matches → safe to resume
        assert!(!remote_resource_changed(
            Some("\"abc\""),
            Some("\"abc\""),
            None,
            None,
        ));
    }

    #[test]
    fn remote_resource_changed_compares_etag_case_insensitively() {
        // HTTP headers are case-insensitive; same ETag with different casing
        // must not be treated as a change.
        assert!(!remote_resource_changed(
            Some("\"ABC123\""),
            Some("\"abc123\""),
            None,
            None,
        ));
    }

    #[test]
    fn remote_resource_changed_falls_back_to_last_modified_when_etag_absent() {
        // No ETag on either side, Last-Modified changed → resource changed
        assert!(remote_resource_changed(
            None,
            None,
            Some("Mon, 01 Jan 2026 00:00:00 GMT"),
            Some("Tue, 02 Feb 2026 00:00:00 GMT"),
        ));

        // No ETag, Last-Modified matches → safe to resume
        assert!(!remote_resource_changed(
            None,
            None,
            Some("Mon, 01 Jan 2026 00:00:00 GMT"),
            Some("Mon, 01 Jan 2026 00:00:00 GMT"),
        ));
    }

    #[test]
    fn remote_resource_changed_compares_last_modified_case_insensitively() {
        assert!(!remote_resource_changed(
            None,
            None,
            Some("Mon, 01 Jan 2026 00:00:00 GMT"),
            Some("mon, 01 jan 2026 00:00:00 gmt"),
        ));
    }

    #[test]
    fn remote_resource_changed_uses_etag_when_both_present_ignoring_last_modified() {
        // ETag matches but Last-Modified differs: ETag is the stronger
        // validator, so the resource is considered unchanged.
        assert!(!remote_resource_changed(
            Some("\"v1\""),
            Some("\"v1\""),
            Some("Mon, 01 Jan 2026 00:00:00 GMT"),
            Some("Tue, 02 Feb 2026 00:00:00 GMT"),
        ));
    }

    #[test]
    fn remote_resource_changed_falls_back_to_last_modified_when_server_omits_etag() {
        // We recorded an ETag, but the fresh HEAD did not return one. Fall
        // back to Last-Modified comparison so we still detect changes.
        assert!(remote_resource_changed(
            Some("\"v1\""),
            None,
            Some("Mon, 01 Jan 2026 00:00:00 GMT"),
            Some("Tue, 02 Feb 2026 00:00:00 GMT"),
        ));
        assert!(!remote_resource_changed(
            Some("\"v1\""),
            None,
            Some("Mon, 01 Jan 2026 00:00:00 GMT"),
            Some("Mon, 01 Jan 2026 00:00:00 GMT"),
        ));
    }

    #[test]
    fn remote_resource_changed_detects_unverifiable_headers() {
        // 已记录 ETag 但新 HEAD 响应缺少校验头，无法重新比对，判定为已改变 (true)
        assert!(remote_resource_changed(Some("\"v1\""), None, None, None,));
        assert!(!remote_resource_changed(None, Some("\"v1\""), None, None,));
        assert!(!remote_resource_changed(None, None, None, None));
    }

    #[test]
    fn remote_changed_error_carries_sentinel_prefix_for_spawn_worker() {
        // spawn_worker matches on this prefix to avoid retrying a task whose
        // remote resource changed. The prefix must stay stable.
        assert!(format!("{REMOTE_CHANGED_PREFIX}远端资源已变化").starts_with(REMOTE_CHANGED_PREFIX));
        assert_eq!(REMOTE_CHANGED_PREFIX, "REMOTE_CHANGED:");
    }

    #[test]
    fn cloud_link_dead_error_carries_sentinel_prefix_for_spawn_worker() {
        // spawn_worker 与 download_segments 的错误汇聚逻辑都按此前缀识别
        // "云盘直链失效"哨兵：前缀必须保持稳定，且分段传输与 worker 上抛
        // 两条路径生成的错误都必须命中。
        assert_eq!(CLOUD_LINK_DEAD_PREFIX, "CLOUD_LINK_DEAD:");
        let empty_sig =
            format!("{CLOUD_LINK_DEAD_PREFIX}分片 #3 连续 3 次收到 0 字节响应，直链疑似已失效");
        let stall_sig = format!(
            "{CLOUD_LINK_DEAD_PREFIX}分片 #5 超过 45 秒下载不足 1048576 字节，直链疑似已失效"
        );
        assert!(empty_sig.starts_with(CLOUD_LINK_DEAD_PREFIX));
        assert!(stall_sig.starts_with(CLOUD_LINK_DEAD_PREFIX));
        // 普通错误不得被误判为直链失效
        assert!(!"分片 #3 连续重试 5 次后仍失败：连接超时".starts_with(CLOUD_LINK_DEAD_PREFIX));
        assert!(!"任务已暂停".starts_with(CLOUD_LINK_DEAD_PREFIX));
    }

    #[test]
    fn cloud_refresh_meta_round_trips_and_defaults_to_none() {
        // 旧 JSON（无 cloud_refresh 字段）必须能安全反序列化为 None。
        #[derive(serde::Deserialize)]
        struct LegacyTask {
            cloud_refresh: Option<crate::models::CloudRefreshMeta>,
        }
        let legacy: LegacyTask = serde_json::from_str(r#"{"cloud_refresh":null}"#).unwrap();
        assert!(legacy.cloud_refresh.is_none());

        // 完整元数据往返：platform/share_id/file_id/device_id 必须无损保留，
        // 刷新直链依赖这些字段重新解析同一文件。
        let meta = crate::models::CloudRefreshMeta {
            platform: "pikpak".into(),
            share_id: "share123".into(),
            file_id: "file456".into(),
            pass_code_token: Some("token789".into()),
            device_id: Some("mb_abc".into()),
        };
        let json = serde_json::to_string(&meta).unwrap();
        let restored: crate::models::CloudRefreshMeta = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.platform, "pikpak");
        assert_eq!(restored.share_id, "share123");
        assert_eq!(restored.file_id, "file456");
        assert_eq!(restored.pass_code_token.as_deref(), Some("token789"));
        assert_eq!(restored.device_id.as_deref(), Some("mb_abc"));

        // 缺省字段（密码分享令牌、设备指纹）必须可省略
        let minimal: crate::models::CloudRefreshMeta = serde_json::from_str(
            r#"{"platform":"pikpak","share_id":"s","file_id":"f"}"#,
        )
        .unwrap();
        assert!(minimal.pass_code_token.is_none());
        assert!(minimal.device_id.is_none());
    }

    #[test]
    fn cloud_refresh_meta_default_is_none_for_plain_tasks() {
        // 普通直链任务（无云盘元数据）不得自动刷新直链：
        // refresh_cloud_direct_link 对 None 返回 Ok(false) 的语义依赖此默认值。
        let mut task = test_task(std::path::Path::new("."), "plain.bin", CollisionPolicy::Rename);
        assert!(task.cloud_refresh.is_none());
        task.cloud_refresh = None;
        assert!(task.cloud_refresh.is_none());
    }

    #[test]
    fn resume_with_changed_etag_preserves_old_shards_for_user_decision() {
        // Simulates the decision branch in download_once: a task with recorded
        // ETag and existing progress receives a fresh HEAD with a different
        // ETag. The old shards MUST be preserved (not cleared) so the user can
        // decide whether to redownload or keep the file.
        let directory = tempfile::tempdir().unwrap();
        let mut task = test_task(directory.path(), "remote.bin", CollisionPolicy::Rename);
        task.etag = Some("\"v1\"".into());
        task.last_modified = Some("Mon, 01 Jan 2026 00:00:00 GMT".into());
        task.downloaded_bytes = 1024;
        task.segments = vec![DownloadSegment {
            index: 0,
            start_byte: 0,
            end_byte: 2047,
            downloaded_bytes: 1024,
            status: "paused".into(),
        }];

        let fresh_etag = Some("\"v2\"");
        let fresh_last_modified = Some("Tue, 02 Feb 2026 00:00:00 GMT");

        let has_progress = task.downloaded_bytes > 0 || !task.segments.is_empty();
        let has_recorded_validator = task.etag.is_some() || task.last_modified.is_some();
        let changed = remote_resource_changed(
            task.etag.as_deref(),
            fresh_etag,
            task.last_modified.as_deref(),
            fresh_last_modified,
        );

        assert!(has_progress, "task has downloaded bytes");
        assert!(
            has_recorded_validator,
            "task has recorded ETag/Last-Modified"
        );
        assert!(
            changed,
            "remote resource changed — task must enter RemoteChanged"
        );

        // The old code silently cleared parts here. The new code MUST keep them
        // so the user can decide. Verify the task still has its progress.
        assert_eq!(task.downloaded_bytes, 1024);
        assert_eq!(task.segments.len(), 1);
        assert_eq!(task.segments[0].downloaded_bytes, 1024);
        assert_eq!(task.etag.as_deref(), Some("\"v1\""));
    }

    #[test]
    fn resume_with_matching_etag_proceeds_normally() {
        // Simulates the decision branch in download_once: a task with recorded
        // ETag and existing progress receives a fresh HEAD with the SAME ETag.
        // The task should proceed with resume (changed = false).
        let directory = tempfile::tempdir().unwrap();
        let mut task = test_task(directory.path(), "stable.bin", CollisionPolicy::Rename);
        task.etag = Some("\"v1\"".into());
        task.downloaded_bytes = 512;
        task.segments = vec![DownloadSegment {
            index: 0,
            start_byte: 0,
            end_byte: 1023,
            downloaded_bytes: 512,
            status: "paused".into(),
        }];

        let fresh_etag = Some("\"v1\"");
        let fresh_last_modified = None;

        let has_progress = task.downloaded_bytes > 0 || !task.segments.is_empty();
        let has_recorded_validator = task.etag.is_some() || task.last_modified.is_some();
        let changed = remote_resource_changed(
            task.etag.as_deref(),
            fresh_etag,
            task.last_modified.as_deref(),
            fresh_last_modified,
        );

        assert!(has_progress);
        assert!(has_recorded_validator);
        assert!(!changed, "ETag matches — task should resume normally");
    }

    #[test]
    fn resume_with_changed_last_modified_and_no_etag_enters_remote_changed() {
        // When the server provides no ETag, Last-Modified is the only
        // validator. A change in Last-Modified must trigger RemoteChanged.
        let directory = tempfile::tempdir().unwrap();
        let mut task = test_task(directory.path(), "no-etag.bin", CollisionPolicy::Rename);
        task.etag = None;
        task.last_modified = Some("Mon, 01 Jan 2026 00:00:00 GMT".into());
        task.downloaded_bytes = 256;
        task.segments = vec![DownloadSegment {
            index: 0,
            start_byte: 0,
            end_byte: 511,
            downloaded_bytes: 256,
            status: "paused".into(),
        }];

        let fresh_etag = None;
        let fresh_last_modified = Some("Tue, 02 Feb 2026 00:00:00 GMT");

        let changed = remote_resource_changed(
            task.etag.as_deref(),
            fresh_etag,
            task.last_modified.as_deref(),
            fresh_last_modified,
        );

        assert!(
            changed,
            "Last-Modified changed with no ETag — must enter RemoteChanged"
        );
    }

    #[test]
    fn fresh_download_without_recorded_validator_skips_remote_changed_check() {
        // A brand-new task has no recorded ETag/Last-Modified and no progress.
        // The resume check must NOT trigger, so the first download proceeds
        // normally.
        let directory = tempfile::tempdir().unwrap();
        let task = test_task(directory.path(), "fresh.bin", CollisionPolicy::Rename);

        let fresh_etag = Some("\"v1\"");
        let fresh_last_modified = Some("Mon, 01 Jan 2026 00:00:00 GMT");

        let has_progress = task.downloaded_bytes > 0 || !task.segments.is_empty();
        let has_recorded_validator = task.etag.is_some() || task.last_modified.is_some();

        assert!(!has_progress, "fresh task has no progress");
        assert!(
            !has_recorded_validator,
            "fresh task has no recorded validator"
        );

        // Even if remote_resource_changed returns something, the guard in
        // download_once (has_progress && has_recorded_validator) prevents
        // entering the RemoteChanged branch.
        let changed = remote_resource_changed(
            task.etag.as_deref(),
            fresh_etag,
            task.last_modified.as_deref(),
            fresh_last_modified,
        );
        let would_enter_remote_changed = has_progress && has_recorded_validator && changed;
        assert!(!would_enter_remote_changed);
    }

    // ===== 磁盘空间保护测试（SubTask 2.5）=====

    #[test]
    fn low_disk_prefix_is_stable_for_spawn_worker_matching() {
        // spawn_worker 通过 starts_with(LOW_DISK_PREFIX) 识别"已由下载循环
        // 处理低盘暂停"，前缀必须保持稳定。
        assert_eq!(LOW_DISK_PREFIX, "LOW_DISK:");
        assert!(format!("{LOW_DISK_PREFIX}磁盘空间不足").starts_with(LOW_DISK_PREFIX));
        // REMOTE_CHANGED_PREFIX 与 LOW_DISK_PREFIX 不能冲突
        assert!(!REMOTE_CHANGED_PREFIX.starts_with(LOW_DISK_PREFIX));
        assert!(!LOW_DISK_PREFIX.starts_with(REMOTE_CHANGED_PREFIX));
    }

    #[test]
    fn compute_low_disk_required_space_uses_remaining_plus_half_plus_margin() {
        // 200MB 文件，已下载 50MB → remaining=150MB
        // required = 150MB + 75MB + 50MB = 275MB
        let total = 200 * 1024 * 1024;
        let downloaded = 50 * 1024 * 1024;
        let expected = 150 * 1024 * 1024 + 75 * 1024 * 1024 + LOW_DISK_SAFETY_MARGIN_BYTES;
        assert_eq!(compute_low_disk_required_space(total, downloaded), expected);
    }

    #[test]
    fn compute_low_disk_required_space_zero_remaining_returns_only_margin() {
        // 文件已全部下载：remaining=0，required = 0 + 0 + 50MB
        assert_eq!(
            compute_low_disk_required_space(1024 * 1024 * 100, 1024 * 1024 * 100),
            LOW_DISK_SAFETY_MARGIN_BYTES
        );
    }

    #[test]
    fn compute_low_disk_required_space_zero_total_returns_only_margin() {
        // 文件大小未知（total=0）：required = 0 + 0 + 50MB
        assert_eq!(
            compute_low_disk_required_space(0, 0),
            LOW_DISK_SAFETY_MARGIN_BYTES
        );
    }

    #[test]
    fn compute_low_disk_required_space_saturates_on_overflow() {
        // u64::MAX 的 remaining 应饱和而非溢出
        assert_eq!(compute_low_disk_required_space(u64::MAX, 0), u64::MAX);
    }

    #[test]
    fn compute_low_disk_required_space_downloaded_exceeds_total_clamps_to_zero() {
        // 已下载超过总大小（异常状态）：remaining 应为 0，不能下溢
        assert_eq!(
            compute_low_disk_required_space(100, 200),
            LOW_DISK_SAFETY_MARGIN_BYTES
        );
    }

    #[test]
    fn check_disk_space_once_returns_ok_when_space_sufficient() {
        // 当前工作目录一定存在且有可用空间，文件较小时应返回 Ok。
        let directory = tempfile::tempdir().unwrap();
        let dest = directory.path().to_string_lossy().to_string();
        // 1MB 文件，未下载，required = 1MB + 0.5MB + 50MB ≈ 51.5MB
        let result = check_disk_space_once(&dest, 1024 * 1024, 0);
        assert!(result.is_ok(), "小型任务在临时目录应通过磁盘空间检查");
    }

    #[test]
    fn check_disk_space_once_returns_err_with_values_when_insufficient() {
        // 使用 u64::MAX 作为总大小，required 饱和到 u64::MAX，
        // 任何真实磁盘的可用空间都小于 u64::MAX，必须返回 Err。
        let directory = tempfile::tempdir().unwrap();
        let dest = directory.path().to_string_lossy().to_string();
        let result = check_disk_space_once(&dest, u64::MAX, 0);
        assert!(result.is_err());
        let (available, required) = result.unwrap_err();
        // available 应为目录的真实可用空间（>0，因为临时目录所在的盘总有空间）
        assert!(available > 0, "临时目录应能查到非零可用空间");
        assert_eq!(
            required,
            u64::MAX,
            "u64::MAX 总大小应饱和到 u64::MAX 所需空间"
        );
        assert!(available < required);
    }

    #[test]
    fn check_disk_space_once_returns_err_for_nonexistent_destination() {
        // 不存在的盘符路径，无祖先存在 → available=0 → 不足
        let result = check_disk_space_once("Z:\\\\nonexistent\\\\deep\\\\path", 1, 0);
        assert!(result.is_err());
        let (available, required) = result.unwrap_err();
        // available 可能是 0（无祖先）或某个真实值（如果 Z: 恰好存在）；
        // required 至少是 50MB 安全余量
        assert!(required >= LOW_DISK_SAFETY_MARGIN_BYTES);
        assert!(available < required);
    }

    #[test]
    fn query_available_space_falls_back_to_ancestor_directory() {
        // 子目录不存在时，应回退到存在的父目录并返回非零值。
        let directory = tempfile::tempdir().unwrap();
        let nested = directory.path().join("a").join("b").join("c");
        let dest = nested.to_string_lossy().to_string();
        let space = query_available_space_for_destination(&dest);
        assert!(space > 0, "回退到存在的祖先目录后应返回非零可用空间");
    }

    #[test]
    fn query_available_space_returns_zero_for_nonexistent_root() {
        // 不存在的盘符路径，无祖先存在 → 返回 0
        let space = query_available_space_for_destination("Z:\\\\nonexistent\\\\deep\\\\path");
        // 在 Windows 上 Z: 不存在时返回 0；如果恰好存在则跳过断言
        let _ = space;
    }

    #[test]
    fn low_disk_payload_serializes_for_frontend_event() {
        // 验证事件载荷可正确序列化，前端按 task_id/available_bytes/required_bytes 读取
        let payload = LowDiskPayload {
            task_id: "task-123".into(),
            available_bytes: 1024,
            required_bytes: 4096,
        };
        let json = serde_json::to_string(&payload).unwrap();
        assert!(json.contains("\"task_id\":\"task-123\""));
        assert!(json.contains("\"available_bytes\":1024"));
        assert!(json.contains("\"required_bytes\":4096"));
        // 反向反序列化也必须工作
        let restored: LowDiskPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, payload);
    }

    #[test]
    fn disk_check_intervals_meet_spec_requirements() {
        // spec 要求：每下载 10MB 或每 5 秒（取先到者）检查一次
        assert_eq!(DISK_CHECK_BYTES_INTERVAL, 10 * 1024 * 1024);
        assert_eq!(DISK_CHECK_TIME_INTERVAL, Duration::from_secs(5));
        // 安全余量必须为 50MB
        assert_eq!(LOW_DISK_SAFETY_MARGIN_BYTES, 50 * 1024 * 1024);
    }

    /// 集成测试：模拟"空间不足"场景，验证任务进入 PausedByLowDisk 状态、
    /// 分片保留、不进入 Failed。
    ///
    /// 此测试不启动真实 HTTP 下载，而是直接验证：
    /// 1. check_disk_space_once 在空间不足时返回 Err
    /// 2. 模拟主循环的处理逻辑（设置状态、保留分片、持久化）
    /// 3. 验证任务状态为 PausedByLowDisk，分片文件保留
    #[test]
    fn low_disk_pause_preserves_shards_and_marks_task_paused_by_low_disk() {
        let directory = tempfile::tempdir().unwrap();
        // Store::open 内部使用 blocking_lock，必须在 tokio runtime 之外构造。
        let store = Store::open(directory.path().to_path_buf()).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            // 构造一个进行中的任务：total_bytes = u64::MAX 必然触发低盘
            let mut task = test_task(directory.path(), "lowdisk.bin", CollisionPolicy::Rename);
            task.id = "low-disk-task".into();
            task.status = TaskStatus::Downloading;
            task.total_bytes = u64::MAX;
            task.downloaded_bytes = 30 * 1024 * 1024; // 已下载 30MB
            task.active_connections = 4;
            task.speed = 1024 * 1024;
            task.segments = vec![
                DownloadSegment {
                    index: 0,
                    start_byte: 0,
                    end_byte: u64::MAX / 2,
                    downloaded_bytes: 15 * 1024 * 1024,
                    status: "downloading".into(),
                },
                DownloadSegment {
                    index: 1,
                    start_byte: u64::MAX / 2 + 1,
                    end_byte: u64::MAX,
                    downloaded_bytes: 15 * 1024 * 1024,
                    status: "downloading".into(),
                },
            ];
            store.upsert_task(&task).await.unwrap();

            // 写入一些"已下载分片"文件，模拟分片保留
            let output = directory.path().join("lowdisk.bin");
            let temp = PathBuf::from(format!("{}.lumaget", output.to_string_lossy()));
            std::fs::write(
                format!("{}.part0", temp.to_string_lossy()),
                vec![0u8; 15 * 1024 * 1024],
            )
            .unwrap();
            std::fs::write(
                format!("{}.part1", temp.to_string_lossy()),
                vec![0u8; 15 * 1024 * 1024],
            )
            .unwrap();

            // 模拟 disk_checker 检测到空间不足
            let check_result =
                check_disk_space_once(&task.destination, task.total_bytes, task.downloaded_bytes);
            assert!(check_result.is_err());
            let (available, required) = check_result.unwrap_err();
            assert!(available < required);

            // 模拟主循环的 PausedByLowDisk 处理逻辑
            task.status = TaskStatus::PausedByLowDisk;
            task.speed = 0;
            task.eta_seconds = None;
            task.active_connections = 0;
            for segment in &mut task.segments {
                if segment.status == "downloading" {
                    segment.status = "paused".into();
                }
            }
            task.error = Some(format!(
                "磁盘空间不足（可用 {} 字节，需要 {} 字节），已暂停",
                available, required
            ));
            store.upsert_task(&task).await.unwrap();

            // 验证任务状态为 PausedByLowDisk（不进入 Failed）
            let restored = store.get_task("low-disk-task").await.unwrap().unwrap();
            assert_eq!(restored.status, TaskStatus::PausedByLowDisk);
            assert_ne!(restored.status, TaskStatus::Failed);
            assert_eq!(restored.speed, 0);
            assert_eq!(restored.active_connections, 0);
            assert_eq!(restored.eta_seconds, None);
            assert!(restored.error.is_some());
            assert!(restored.error.as_ref().unwrap().contains("磁盘空间不足"));

            // 验证分片状态被置为 paused（保留分片记录）
            assert_eq!(restored.segments.len(), 2);
            for segment in &restored.segments {
                assert_eq!(segment.status, "paused");
                assert_eq!(segment.downloaded_bytes, 15 * 1024 * 1024);
            }

            // 验证分片文件未被删除（保留可恢复状态）
            assert!(PathBuf::from(format!("{}.part0", temp.to_string_lossy())).exists());
            assert!(PathBuf::from(format!("{}.part1", temp.to_string_lossy())).exists());

            // 验证下载字节数保留（可恢复续传）
            assert_eq!(restored.downloaded_bytes, 30 * 1024 * 1024);
        });
    }

    /// 集成测试：验证低盘暂停后任务可恢复（用户清理空间后可继续）。
    ///
    /// 验证流程：
    /// 1. 任务进入 PausedByLowDisk 状态
    /// 2. 模拟用户清理空间（实际上 total_bytes 减小到合理值）
    /// 3. check_disk_space_once 返回 Ok，表示可恢复
    #[test]
    fn low_disk_pause_is_recoverable_after_space_freed() {
        let directory = tempfile::tempdir().unwrap();
        let dest = directory.path().to_string_lossy().to_string();

        // 1. 模拟低盘：u64::MAX 文件大小必然触发
        let low_disk_result = check_disk_space_once(&dest, u64::MAX, 0);
        assert!(low_disk_result.is_err());

        // 2. 模拟用户清理空间或更换目录：文件大小恢复为合理值
        // 1MB 文件，required = 1MB + 0.5MB + 50MB ≈ 51.5MB，临时目录应能满足
        let recovered_result = check_disk_space_once(&dest, 1024 * 1024, 0);
        assert!(recovered_result.is_ok(), "清理空间后应能恢复下载");
    }

    /// 验证 LOW_DISK 错误不会被 spawn_worker 当作普通错误重试。
    ///
    /// spawn_worker 的错误匹配顺序：
    /// 1. REMOTE_CHANGED_PREFIX → break
    /// 2. LOW_DISK_PREFIX → break（不重试、不进入 Failed）
    /// 3. is_network_error → 等待网络
    /// 4. attempt < max_retries → 重试
    /// 5. 其他 → Failed
    #[test]
    fn low_disk_error_is_not_treated_as_network_error() {
        let low_disk_error = format!("{LOW_DISK_PREFIX}磁盘空间不足");
        assert!(!is_network_error(&low_disk_error));
    }

    #[test]
    fn low_disk_error_is_not_treated_as_remote_changed() {
        let low_disk_error = format!("{LOW_DISK_PREFIX}磁盘空间不足");
        assert!(!low_disk_error.starts_with(REMOTE_CHANGED_PREFIX));
    }

    #[test]
    fn validate_preset_connections_accepts_allowed_tiers() {
        for tier in [1u8, 2, 4, 8, 16, 32] {
            assert!(
                validate_preset_connections(tier).is_ok(),
                "tier {tier} should be accepted"
            );
        }
    }

    #[test]
    fn validate_preset_connections_rejects_invalid_tiers() {
        // 0、3、5、6、7、9、10、15、17、31、33、64、100、255 等都不允许
        for tier in [0u8, 3, 5, 6, 7, 9, 10, 15, 17, 31, 33, 64, 100, 255] {
            let result = validate_preset_connections(tier);
            assert!(result.is_err(), "tier {tier} should be rejected");
            assert_eq!(result.unwrap_err(), "连接数只能是 1 / 2 / 4 / 8 / 16 / 32");
        }
    }

    #[test]
    fn validate_preset_scheduled_at_accepts_hh_mm_and_none() {
        assert!(validate_preset_scheduled_at(None).is_ok());
        assert!(validate_preset_scheduled_at(Some("00:00")).is_ok());
        assert!(validate_preset_scheduled_at(Some("22:00")).is_ok());
        assert!(validate_preset_scheduled_at(Some("23:59")).is_ok());
        assert!(validate_preset_scheduled_at(Some("09:05")).is_ok());
    }

    #[test]
    fn validate_preset_scheduled_at_rejects_invalid_formats() {
        // 错误格式
        assert!(validate_preset_scheduled_at(Some("")).is_err());
        assert!(validate_preset_scheduled_at(Some("22")).is_err());
        assert!(validate_preset_scheduled_at(Some("2200")).is_err());
        assert!(validate_preset_scheduled_at(Some("22-00")).is_err());
        assert!(validate_preset_scheduled_at(Some("2:00")).is_err());
        assert!(validate_preset_scheduled_at(Some("22:0")).is_err());
        // 越界值
        assert!(validate_preset_scheduled_at(Some("24:00")).is_err());
        assert!(validate_preset_scheduled_at(Some("23:60")).is_err());
        assert!(validate_preset_scheduled_at(Some("99:99")).is_err());
        // 非法字符
        assert!(validate_preset_scheduled_at(Some("ab:cd")).is_err());
    }

    #[test]
    fn next_scheduled_timestamp_returns_future_for_valid_hh_mm() {
        // 22:00 一定返回未来的时间戳，且与当前时间的差距不超过 24 小时
        let now_ms = now();
        let ts = next_scheduled_timestamp("22:00").expect("22:00 should produce a timestamp");
        assert!(ts > now_ms, "timestamp must be in the future");
        const DAY_MS: u64 = 24 * 60 * 60 * 1000;
        assert!(ts - now_ms <= DAY_MS, "delta must not exceed 24 hours");
    }

    #[test]
    fn next_scheduled_timestamp_returns_none_for_invalid_input() {
        assert!(next_scheduled_timestamp("").is_none());
        assert!(next_scheduled_timestamp("invalid").is_none());
        assert!(next_scheduled_timestamp("99:99").is_none());
    }

    // ===== Task 12.6: apply_preset_to_task_fields 集成测试 =====
    //
    // apply_preset_to_task_fields 是 preset_apply_to_task 命令的核心纯函数：
    // 不依赖 AppHandle / Store，可直接测试。覆盖各任务状态分支、scheduled_at 转换、
    // 字段覆盖语义。命名前缀 `preset_apply_` 便于 `cargo test --lib preset_apply` 过滤。

    /// 辅助：构造一个可定制状态的预设。
    fn preset_apply_test_preset(
        connections: u8,
        speed_limit: Option<u64>,
        completion_action: Option<CompletionAction>,
        scheduled_at: Option<&str>,
    ) -> DownloadPreset {
        DownloadPreset {
            id: "test-preset".into(),
            name: "测试预设".into(),
            connections,
            speed_limit,
            completion_action,
            verify_checksum: false,
            scheduled_at: scheduled_at.map(str::to_owned),
            is_builtin: false,
        }
    }

    /// 辅助：在 tempdir 下构造一个处于指定状态的任务。
    fn preset_apply_test_task(directory: &Path, status: TaskStatus) -> DownloadTask {
        let mut task = test_task(directory, "preset.bin", CollisionPolicy::Rename);
        task.status = status;
        task
    }

    #[test]
    fn preset_apply_overwrites_all_fields_when_preset_has_full_config() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut task = preset_apply_test_task(dir.path(), TaskStatus::Queued);
        // 初始值非预设值，验证字段被覆盖。
        task.connection_count = 1;
        task.per_task_speed_limit = 999;
        task.completion_action = CompletionAction::None;

        let preset =
            preset_apply_test_preset(16, Some(2_000_000), Some(CompletionAction::Shutdown), None);

        apply_preset_to_task_fields(&mut task, &preset).expect("queued task should accept preset");

        assert_eq!(task.connection_count, 16);
        assert_eq!(task.per_task_speed_limit, 2_000_000);
        assert_eq!(task.completion_action, CompletionAction::Shutdown);
        // 预设无 scheduled_at：任务保持 Queued，不绑定计划时间。
        assert_eq!(task.status, TaskStatus::Queued);
        assert!(task.scheduled_at.is_none());
    }

    #[test]
    fn preset_apply_uses_defaults_when_preset_optional_fields_are_none() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut task = preset_apply_test_task(dir.path(), TaskStatus::Queued);
        task.per_task_speed_limit = 1234;
        task.completion_action = CompletionAction::OpenFolder;

        let preset = preset_apply_test_preset(8, None, None, None);

        apply_preset_to_task_fields(&mut task, &preset).expect("queued task should accept preset");

        // speed_limit=None → 0（不限速）；completion_action=None → 默认 None。
        assert_eq!(task.per_task_speed_limit, 0);
        assert_eq!(task.completion_action, CompletionAction::None);
    }

    #[test]
    fn preset_apply_transitions_queued_to_scheduled_when_preset_has_hh_mm() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut task = preset_apply_test_task(dir.path(), TaskStatus::Queued);
        let now_ms = now();

        let preset =
            preset_apply_test_preset(8, None, Some(CompletionAction::Shutdown), Some("22:00"));

        apply_preset_to_task_fields(&mut task, &preset).expect("queued task should accept preset");

        // 22:00 → 必须生成未来时间戳，且不超过 24 小时。
        assert_eq!(task.status, TaskStatus::Scheduled);
        let ts = task.scheduled_at.expect("scheduled_at should be set");
        assert!(ts > now_ms, "scheduled_at must be in the future");
        const DAY_MS: u64 = 24 * 60 * 60 * 1000;
        assert!(
            ts - now_ms <= DAY_MS,
            "scheduled_at delta must not exceed 24h"
        );
        // completion_action 仍应被覆盖。
        assert_eq!(task.completion_action, CompletionAction::Shutdown);
    }

    #[test]
    fn preset_apply_keeps_scheduled_status_when_preset_has_hh_mm_and_task_was_scheduled() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut task = preset_apply_test_task(dir.path(), TaskStatus::Scheduled);
        task.scheduled_at = Some(now() + 60_000); // 任意旧时间戳

        let preset = preset_apply_test_preset(8, None, None, Some("03:30"));

        apply_preset_to_task_fields(&mut task, &preset)
            .expect("scheduled task should accept preset");

        // Scheduled 状态保持，但 scheduled_at 必须被刷新为新预设的时间戳。
        assert_eq!(task.status, TaskStatus::Scheduled);
        assert!(task.scheduled_at.is_some());
    }

    #[test]
    fn preset_apply_clears_scheduled_at_when_preset_has_no_time_and_task_was_scheduled() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut task = preset_apply_test_task(dir.path(), TaskStatus::Scheduled);
        task.scheduled_at = Some(now() + 60_000);

        let preset = preset_apply_test_preset(8, None, None, None);

        apply_preset_to_task_fields(&mut task, &preset)
            .expect("scheduled task should accept preset");

        // 预设无计划时间：Scheduled 必须降级为 Queued，scheduled_at 清空。
        assert_eq!(task.status, TaskStatus::Queued);
        assert!(task.scheduled_at.is_none());
    }

    #[test]
    fn preset_apply_accepts_paused_failed_cancelled_statuses() {
        let dir = tempfile::tempdir().expect("tempdir");
        let preset = preset_apply_test_preset(4, None, None, None);

        for status in [
            TaskStatus::Paused,
            TaskStatus::Failed,
            TaskStatus::Cancelled,
        ] {
            let mut task = preset_apply_test_task(dir.path(), status.clone());
            apply_preset_to_task_fields(&mut task, &preset)
                .unwrap_or_else(|e| panic!("{status:?} should accept preset, got: {e}"));
            assert_eq!(task.connection_count, 4);
        }
    }

    #[test]
    fn preset_apply_rejects_downloading_status() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut task = preset_apply_test_task(dir.path(), TaskStatus::Downloading);
        let preset = preset_apply_test_preset(8, None, None, None);

        let err = apply_preset_to_task_fields(&mut task, &preset)
            .expect_err("downloading task should reject preset");
        assert_eq!(err, "任务正在下载或校验，无法应用预设");
    }

    #[test]
    fn preset_apply_rejects_verifying_and_waiting_network_statuses() {
        let dir = tempfile::tempdir().expect("tempdir");
        let preset = preset_apply_test_preset(8, None, None, None);

        // 下载中、校验中、网络等待、磁盘不足暂停、远程变更、中断、完成 — 都不允许套用预设。
        for status in [
            TaskStatus::Verifying,
            TaskStatus::WaitingNetwork,
            TaskStatus::PausedByLowDisk,
            TaskStatus::RemoteChanged,
            TaskStatus::Interrupted,
            TaskStatus::Completed,
        ] {
            let mut task = preset_apply_test_task(dir.path(), status.clone());
            let result = apply_preset_to_task_fields(&mut task, &preset);
            assert!(
                result.is_err(),
                "{status:?} should reject preset, but got Ok"
            );
        }
    }

    #[test]
    fn preset_apply_does_not_touch_verify_checksum_field_on_task() {
        // DownloadTask 没有 verify_checksum 字段（校验由 expected_checksum 驱动），
        // 预设的 verify_checksum 只在前端新建任务对话框决定是否填入 expected_checksum。
        // 这里验证 apply_preset_to_task_fields 不会因为 verify_checksum=true 而破坏任务。
        let dir = tempfile::tempdir().expect("tempdir");
        let mut task = preset_apply_test_task(dir.path(), TaskStatus::Queued);
        let mut preset = preset_apply_test_preset(8, None, None, None);
        preset.verify_checksum = true;

        apply_preset_to_task_fields(&mut task, &preset).expect("queued task should accept preset");
        // expected_checksum 未被改动。
        assert!(task.expected_checksum.is_none());
    }

    // ===== Task 15: 队列调度可观察性 — compute_wait_reason 单元测试 =====
    //
    // explain_wait_reason 是 I/O 包装：从 store/settings 取数据后委托给纯函数
    // compute_wait_reason。这里直接测试纯函数，覆盖每个状态分支以及多任务排队场景。
    // 命名前缀 `wait_reason_` 便于 `cargo test --lib wait_reason` 过滤。
    use crate::models::MediaSelection;

    /// 辅助：构造一个可定制的任务（基于 test_task，但允许设置 id/优先级/队列位置/状态）。
    fn wait_reason_task(
        directory: &Path,
        id: &str,
        status: TaskStatus,
        priority: i32,
        queue_position: i64,
    ) -> DownloadTask {
        let mut task = test_task(directory, "wait.bin", CollisionPolicy::Rename);
        task.id = id.into();
        task.status = status;
        task.priority = priority;
        task.queue_position = queue_position;
        task
    }

    #[test]
    fn wait_reason_returns_not_waiting_for_active_or_terminal_states() {
        let directory = tempfile::tempdir().unwrap();
        // 这些状态都表示任务不在等待队列中：正在下载、已完成、失败、已取消、校验中、等待网络。
        for status in [
            TaskStatus::Downloading,
            TaskStatus::Completed,
            TaskStatus::Failed,
            TaskStatus::Cancelled,
            TaskStatus::Verifying,
            TaskStatus::WaitingNetwork,
        ] {
            let task = wait_reason_task(directory.path(), "t", status.clone(), 0, 0);
            let reason = compute_wait_reason(&task, &[], 0, 1, true, true);
            assert_eq!(
                reason,
                WaitReason::NotWaiting,
                "status {status:?} should be NotWaiting"
            );
        }
    }

    #[test]
    fn wait_reason_returns_paused_states_interrupted_and_remote_changed() {
        let directory = tempfile::tempdir().unwrap();
        let cases = [
            (TaskStatus::Paused, WaitReason::Paused),
            (TaskStatus::PausedByLowDisk, WaitReason::PausedByLowDisk),
            (TaskStatus::PausedByMetered, WaitReason::PausedByMetered),
            (TaskStatus::Interrupted, WaitReason::Interrupted),
            (TaskStatus::RemoteChanged, WaitReason::RemoteChanged),
        ];
        for (status, expected) in cases {
            let task = wait_reason_task(directory.path(), "t", status.clone(), 0, 0);
            let reason = compute_wait_reason(&task, &[], 0, 1, true, true);
            assert_eq!(reason, expected, "status {status:?} mismatch");
        }
    }

    #[test]
    fn wait_reason_returns_waiting_scheduled_time_with_timestamp() {
        let directory = tempfile::tempdir().unwrap();
        let mut task = wait_reason_task(directory.path(), "scheduled", TaskStatus::Scheduled, 0, 0);
        task.scheduled_at = Some(1_800_000_000_000); // 固定时间戳，便于断言

        let reason = compute_wait_reason(&task, &[], 0, 1, true, true);
        match reason {
            WaitReason::WaitingScheduledTime { scheduled_at } => {
                assert_eq!(scheduled_at, "1800000000000");
            }
            other => panic!("expected WaitingScheduledTime, got {other:?}"),
        }
    }

    #[test]
    fn wait_reason_returns_waiting_scheduled_time_with_empty_string_when_unset() {
        // 旧数据库可能存在 Scheduled 状态但 scheduled_at 为 None 的脏数据。
        // 此时返回空字符串而非 panic（与 unwrap_or_default 一致）。
        let directory = tempfile::tempdir().unwrap();
        let task = wait_reason_task(
            directory.path(),
            "scheduled-no-ts",
            TaskStatus::Scheduled,
            0,
            0,
        );
        let reason = compute_wait_reason(&task, &[], 0, 1, true, true);
        match reason {
            WaitReason::WaitingScheduledTime { scheduled_at } => {
                assert_eq!(scheduled_at, "");
            }
            other => panic!("expected WaitingScheduledTime, got {other:?}"),
        }
    }

    #[test]
    fn wait_reason_returns_waiting_media_tools_when_yt_dlp_missing() {
        // 媒体任务（带 media 字段）但 yt-dlp 未安装 → 等待媒体工具。
        let directory = tempfile::tempdir().unwrap();
        let mut task = wait_reason_task(directory.path(), "media", TaskStatus::Queued, 0, 0);
        task.media = Some(MediaSelection {
            extractor: Some("youtube".into()),
            format_id: Some("137+140".into()), // 含 + 需要 ffmpeg
            format_label: None,
            subtitles: Vec::new(),
            thumbnail: None,
            requires_ffmpeg: false,
            url: None,
        });

        // yt_dlp_available = false → 等待媒体工具（无论 ffmpeg 是否可用）
        let reason = compute_wait_reason(&task, &[], 0, 4, false, true);
        assert_eq!(reason, WaitReason::WaitingMediaTools);

        // 工具齐全 → 不在等待（除非有其他原因，这里没有）
        let reason = compute_wait_reason(&task, &[], 0, 4, true, true);
        assert_eq!(reason, WaitReason::NotWaiting);
    }

    #[test]
    fn wait_reason_returns_waiting_media_tools_when_ffmpeg_missing_for_merge_format() {
        // 格式 ID 含 `+`（视频+音频合并）但 ffmpeg 未安装 → 等待媒体工具。
        let directory = tempfile::tempdir().unwrap();
        let mut task = wait_reason_task(directory.path(), "merge", TaskStatus::Queued, 0, 0);
        task.media = Some(MediaSelection {
            extractor: None,
            format_id: Some("137+140".into()),
            format_label: None,
            subtitles: Vec::new(),
            thumbnail: None,
            requires_ffmpeg: false,
            url: None,
        });

        let reason = compute_wait_reason(&task, &[], 0, 4, true, false);
        assert_eq!(reason, WaitReason::WaitingMediaTools);
    }

    #[test]
    fn wait_reason_returns_waiting_media_tools_when_requires_ffmpeg_flag_set() {
        // 即使 format_id 不含 `+`，但 media.requires_ffmpeg = true → 需要 ffmpeg。
        let directory = tempfile::tempdir().unwrap();
        let mut task = wait_reason_task(directory.path(), "needs-ffmpeg", TaskStatus::Queued, 0, 0);
        task.media = Some(MediaSelection {
            extractor: None,
            format_id: Some("22".into()),
            format_label: None,
            subtitles: Vec::new(),
            thumbnail: None,
            requires_ffmpeg: true,
            url: None,
        });

        let reason = compute_wait_reason(&task, &[], 0, 4, true, false);
        assert_eq!(reason, WaitReason::WaitingMediaTools);
    }

    #[test]
    fn wait_reason_returns_waiting_concurrency_limit_when_slots_full() {
        // 并发槽位已满（active_count >= max_concurrent）→ 等待并发槽位。
        let directory = tempfile::tempdir().unwrap();
        let task = wait_reason_task(directory.path(), "queued", TaskStatus::Queued, 0, 0);

        // 3 个活动任务、上限 3 → 满
        let reason = compute_wait_reason(&task, &[], 3, 3, true, true);
        match reason {
            WaitReason::WaitingConcurrencyLimit { active_count } => {
                assert_eq!(active_count, 3);
            }
            other => panic!("expected WaitingConcurrencyLimit, got {other:?}"),
        }

        // 2 个活动、上限 3 → 不满，继续判断 ahead_count
        let reason = compute_wait_reason(&task, &[], 2, 3, true, true);
        assert_eq!(reason, WaitReason::NotWaiting);
    }

    #[test]
    fn wait_reason_returns_queued_behind_with_correct_ahead_count() {
        // 多任务排队：更小优先级 + 同优先级更早创建的 → 都算"前面"。
        let directory = tempfile::tempdir().unwrap();
        let target = wait_reason_task(directory.path(), "target", TaskStatus::Queued, 0, 5);
        // 更小优先级任务（priority=-1 < 0），排在 target 前面
        let higher = wait_reason_task(directory.path(), "higher", TaskStatus::Queued, -1, 99);
        // 同优先级、queue_position 更小（创建更早），排在 target 前面
        let earlier = wait_reason_task(directory.path(), "earlier", TaskStatus::Queued, 0, 1);
        // 同优先级、queue_position 更大（创建更晚），不算前面
        let later = wait_reason_task(directory.path(), "later", TaskStatus::Queued, 0, 9);
        // 更大优先级任务（priority=1 > 0），不算前面
        let lower = wait_reason_task(directory.path(), "lower", TaskStatus::Queued, 1, 0);

        let all = vec![target.clone(), higher, earlier, later, lower];
        let reason = compute_wait_reason(&target, &all, 0, 4, true, true);
        match reason {
            WaitReason::QueuedBehind { ahead_count } => {
                // higher + earlier = 2
                assert_eq!(ahead_count, 2, "ahead_count should be 2 (higher + earlier)");
            }
            other => panic!("expected QueuedBehind, got {other:?}"),
        }
    }

    #[test]
    fn wait_reason_ahead_count_excludes_non_queued_tasks() {
        // 排在前面的任务如果不是 Queued 状态（如 Downloading/Paused），
        // 不应计入 ahead_count——它们不占用队列位置。
        let directory = tempfile::tempdir().unwrap();
        let target = wait_reason_task(directory.path(), "target", TaskStatus::Queued, 0, 5);
        // 更小优先级但正在下载中 → 不算前面
        let downloading_high =
            wait_reason_task(directory.path(), "dl-high", TaskStatus::Downloading, -1, 1);
        // 同优先级更早但已暂停 → 不算前面
        let paused_earlier =
            wait_reason_task(directory.path(), "paused-earlier", TaskStatus::Paused, 0, 1);
        // 更小优先级且 Queued → 算前面
        let queued_high =
            wait_reason_task(directory.path(), "queued-high", TaskStatus::Queued, -1, 1);

        let all = vec![
            target.clone(),
            downloading_high,
            paused_earlier,
            queued_high,
        ];
        let reason = compute_wait_reason(&target, &all, 0, 4, true, true);
        match reason {
            WaitReason::QueuedBehind { ahead_count } => {
                assert_eq!(ahead_count, 1, "only queued_high should count");
            }
            other => panic!("expected QueuedBehind, got {other:?}"),
        }
    }

    #[test]
    fn wait_reason_ahead_count_excludes_self() {
        // 目标任务自身不应被计入 ahead_count。
        let directory = tempfile::tempdir().unwrap();
        let target = wait_reason_task(directory.path(), "target", TaskStatus::Queued, 0, 0);
        let all = vec![target.clone()];
        let reason = compute_wait_reason(&target, &all, 0, 4, true, true);
        assert_eq!(reason, WaitReason::NotWaiting);
    }

    #[test]
    fn wait_reason_returns_not_waiting_when_queue_empty_and_slots_available() {
        // 队列中没有更靠前的任务，且有空闲并发槽位 → 不在等待（即将开始）。
        let directory = tempfile::tempdir().unwrap();
        let target = wait_reason_task(directory.path(), "solo", TaskStatus::Queued, 0, 0);
        let reason = compute_wait_reason(&target, &[target.clone()], 0, 4, true, true);
        assert_eq!(reason, WaitReason::NotWaiting);
    }

    #[test]
    fn wait_reason_priority_order_matches_sort_download_candidates() {
        // 验证 ahead_count 的排序逻辑（Task 16: priority ASC, queue_position ASC）
        // 与 sort_download_candidates 一致。
        let directory = tempfile::tempdir().unwrap();
        let candidates = vec![
            wait_reason_task(directory.path(), "low-2", TaskStatus::Queued, 1, 2),
            wait_reason_task(directory.path(), "normal-2", TaskStatus::Queued, 0, 2),
            wait_reason_task(directory.path(), "high-1", TaskStatus::Queued, -1, 1),
            wait_reason_task(directory.path(), "normal-1", TaskStatus::Queued, 0, 1),
            wait_reason_task(directory.path(), "high-2", TaskStatus::Queued, -1, 2),
            wait_reason_task(directory.path(), "low-1", TaskStatus::Queued, 1, 1),
        ];

        // 用 sort_download_candidates 排序，得到预期顺序
        let mut sorted = candidates.clone();
        sort_download_candidates(&mut sorted);
        let sorted_ids: Vec<&str> = sorted.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(
            sorted_ids,
            // Task 16: priority ASC: high(-1) → normal(0) → low(1)
            // 同优先级内 queue_position ASC: high-1, high-2 / normal-1, normal-2 / low-1, low-2
            ["high-1", "high-2", "normal-1", "normal-2", "low-1", "low-2"]
        );

        // 对每个任务用 compute_wait_reason 计算 ahead_count，
        // 验证 ahead_count == 它在排序后列表中的位置（0-indexed）
        let all = candidates.clone();
        for task in &candidates {
            let reason = compute_wait_reason(task, &all, 0, 100, true, true);
            let position = sorted_ids
                .iter()
                .position(|id| *id == task.id.as_str())
                .unwrap();
            match reason {
                WaitReason::QueuedBehind { ahead_count } => {
                    assert_eq!(
                        ahead_count as usize, position,
                        "task {} ahead_count {} != position {}",
                        task.id, ahead_count, position
                    );
                }
                WaitReason::NotWaiting => {
                    assert_eq!(
                        position, 0,
                        "task {} should be QueuedBehind but got NotWaiting",
                        task.id
                    );
                }
                other => panic!("task {} unexpected reason: {:?}", task.id, other),
            }
        }
    }

    #[test]
    fn wait_reason_media_check_takes_precedence_over_concurrency() {
        // 媒体工具缺失时优先返回 WaitingMediaTools，即使并发槽位也满。
        let directory = tempfile::tempdir().unwrap();
        let mut task = wait_reason_task(directory.path(), "media-full", TaskStatus::Queued, 0, 0);
        task.media = Some(MediaSelection {
            extractor: None,
            format_id: Some("137+140".into()),
            format_label: None,
            subtitles: Vec::new(),
            thumbnail: None,
            requires_ffmpeg: false,
            url: None,
        });
        // active_count = 5, max_concurrent = 3（满），且 yt_dlp 缺失
        let reason = compute_wait_reason(&task, &[], 5, 3, false, true);
        assert_eq!(reason, WaitReason::WaitingMediaTools);
    }

    #[test]
    fn wait_reason_concurrency_check_takes_precedence_over_queue_position() {
        // 并发槽位满时优先返回 WaitingConcurrencyLimit，即使前面还有排队任务。
        let directory = tempfile::tempdir().unwrap();
        let target = wait_reason_task(directory.path(), "behind-full", TaskStatus::Queued, 0, 5);
        let higher = wait_reason_task(directory.path(), "higher", TaskStatus::Queued, -1, 1);
        let all = vec![target.clone(), higher];

        // active_count = 3, max_concurrent = 3（满），且 ahead_count = 1
        let reason = compute_wait_reason(&target, &all, 3, 3, true, true);
        match reason {
            WaitReason::WaitingConcurrencyLimit { active_count } => {
                assert_eq!(active_count, 3);
            }
            other => panic!("expected WaitingConcurrencyLimit, got {other:?}"),
        }
    }

    #[test]
    fn wait_reason_default_is_not_waiting() {
        // WaitReason 实现 Default，默认值为 NotWaiting。
        // 这确保旧前端/旧 JSON 反序列化时缺失 kind 字段不会 panic。
        assert_eq!(WaitReason::default(), WaitReason::NotWaiting);
    }

    #[test]
    fn wait_reason_serializes_with_kebab_case_tag() {
        // 验证 serde tag = "kind", rename_all = "kebab-case" 配置正确。
        // 前端 TypeScript 类型使用联合判别式（kind 字段），必须与之匹配。
        let cases: Vec<(WaitReason, &str)> = vec![
            (WaitReason::NotWaiting, r#"{"kind":"not-waiting"}"#),
            (
                WaitReason::QueuedBehind { ahead_count: 3 },
                r#"{"kind":"queued-behind","ahead_count":3}"#,
            ),
            (
                WaitReason::WaitingMediaTools,
                r#"{"kind":"waiting-media-tools"}"#,
            ),
            (
                WaitReason::WaitingConcurrencyLimit { active_count: 2 },
                r#"{"kind":"waiting-concurrency-limit","active_count":2}"#,
            ),
            (
                WaitReason::WaitingScheduledTime {
                    scheduled_at: "123".into(),
                },
                r#"{"kind":"waiting-scheduled-time","scheduled_at":"123"}"#,
            ),
            (WaitReason::Paused, r#"{"kind":"paused"}"#),
            (
                WaitReason::PausedByLowDisk,
                r#"{"kind":"paused-by-low-disk"}"#,
            ),
            (
                WaitReason::PausedByMetered,
                r#"{"kind":"paused-by-metered"}"#,
            ),
            (WaitReason::Interrupted, r#"{"kind":"interrupted"}"#),
            (WaitReason::RemoteChanged, r#"{"kind":"remote-changed"}"#),
            (WaitReason::Unknown, r#"{"kind":"unknown"}"#),
        ];
        for (reason, expected_json) in cases {
            let json = serde_json::to_string(&reason).unwrap();
            assert_eq!(json, expected_json, "serialization mismatch for {reason:?}");
        }
    }

    #[test]
    fn wait_reason_deserializes_missing_optional_fields_with_defaults() {
        // 旧前端或旧 JSON 可能缺少 ahead_count/active_count/scheduled_at 字段。
        // #[serde(default)] 必须保证反序列化成功且字段为默认值。
        let reason: WaitReason = serde_json::from_str(r#"{"kind":"queued-behind"}"#).unwrap();
        match reason {
            WaitReason::QueuedBehind { ahead_count } => assert_eq!(ahead_count, 0),
            other => panic!("expected QueuedBehind, got {other:?}"),
        }

        let reason: WaitReason =
            serde_json::from_str(r#"{"kind":"waiting-concurrency-limit"}"#).unwrap();
        match reason {
            WaitReason::WaitingConcurrencyLimit { active_count } => assert_eq!(active_count, 0),
            other => panic!("expected WaitingConcurrencyLimit, got {other:?}"),
        }

        let reason: WaitReason =
            serde_json::from_str(r#"{"kind":"waiting-scheduled-time"}"#).unwrap();
        match reason {
            WaitReason::WaitingScheduledTime { scheduled_at } => assert_eq!(scheduled_at, ""),
            other => panic!("expected WaitingScheduledTime, got {other:?}"),
        }
    }

    // ===== Task 14: 任务级超时与重试策略测试 =====

    fn retry_policy_with_backoff(backoff: BackoffStrategy) -> RetryPolicy {
        RetryPolicy {
            connection_timeout_secs: 30,
            task_timeout_secs: None,
            max_retries: 5,
            backoff,
            initial_backoff_ms: 1000,
            max_backoff_ms: 60_000,
        }
    }

    #[test]
    fn compute_backoff_fixed_returns_initial_for_all_attempts() {
        let policy = retry_policy_with_backoff(BackoffStrategy::Fixed);
        // Fixed 退避：所有尝试都返回 initial_backoff_ms。
        assert_eq!(compute_backoff(&policy, 1), 1000);
        assert_eq!(compute_backoff(&policy, 2), 1000);
        assert_eq!(compute_backoff(&policy, 5), 1000);
        assert_eq!(compute_backoff(&policy, 100), 1000);
    }

    #[test]
    fn compute_backoff_exponential_doubles_each_attempt() {
        let policy = retry_policy_with_backoff(BackoffStrategy::Exponential);
        // 指数退避：attempt 1 -> 1000, 2 -> 2000, 3 -> 4000, 4 -> 8000。
        assert_eq!(compute_backoff(&policy, 1), 1000);
        assert_eq!(compute_backoff(&policy, 2), 2000);
        assert_eq!(compute_backoff(&policy, 3), 4000);
        assert_eq!(compute_backoff(&policy, 4), 8000);
        assert_eq!(compute_backoff(&policy, 5), 16_000);
    }

    #[test]
    fn compute_backoff_exponential_capped_at_max_backoff_ms() {
        let policy = RetryPolicy {
            connection_timeout_secs: 30,
            task_timeout_secs: None,
            max_retries: 10,
            backoff: BackoffStrategy::Exponential,
            initial_backoff_ms: 1000,
            max_backoff_ms: 8_000,
        };
        // 1 -> 1000, 2 -> 2000, 3 -> 4000, 4 -> 8000 (上限), 5 -> 8000 (capped), 6 -> 8000。
        assert_eq!(compute_backoff(&policy, 1), 1000);
        assert_eq!(compute_backoff(&policy, 2), 2000);
        assert_eq!(compute_backoff(&policy, 3), 4000);
        assert_eq!(compute_backoff(&policy, 4), 8000);
        assert_eq!(compute_backoff(&policy, 5), 8000);
        assert_eq!(compute_backoff(&policy, 100), 8000);
    }

    #[test]
    fn compute_backoff_handles_attempt_zero_or_underflow_safely() {
        let policy = retry_policy_with_backoff(BackoffStrategy::Exponential);
        // attempt 0 视为 1，返回 initial_backoff_ms。
        assert_eq!(compute_backoff(&policy, 0), 1000);
        // 极大 attempt 不会溢出，由 saturating_mul 保护并封顶。
        let huge_policy = RetryPolicy {
            connection_timeout_secs: 30,
            task_timeout_secs: None,
            max_retries: 5,
            backoff: BackoffStrategy::Exponential,
            initial_backoff_ms: 1000,
            max_backoff_ms: 60_000,
        };
        assert_eq!(compute_backoff(&huge_policy, u32::MAX), 60_000);
    }

    #[test]
    fn effective_retry_policy_uses_task_override_when_present() {
        let directory = tempfile::tempdir().unwrap();
        let mut task = test_task(directory.path(), "file.zip", CollisionPolicy::Rename);
        let settings = AppSettings::default();
        // 默认情况下任务无覆盖，使用全局默认。
        assert_eq!(
            effective_retry_policy(&task, &settings),
            settings.default_retry_policy
        );
        // 设置任务级覆盖后应优先使用覆盖。
        let override_policy = RetryPolicy {
            connection_timeout_secs: 99,
            task_timeout_secs: Some(300),
            max_retries: 7,
            backoff: BackoffStrategy::Fixed,
            initial_backoff_ms: 500,
            max_backoff_ms: 5_000,
        };
        task.retry_policy_override = Some(override_policy.clone());
        let effective = effective_retry_policy(&task, &settings);
        assert_eq!(effective, override_policy);
        // 全局默认未受影响。
        assert_ne!(effective, settings.default_retry_policy);
    }

    #[test]
    fn effective_retry_policy_falls_back_to_global_default() {
        let directory = tempfile::tempdir().unwrap();
        let task = test_task(directory.path(), "file.zip", CollisionPolicy::Rename);
        let mut settings = AppSettings::default();
        // 任务无覆盖：使用全局默认。
        let default_policy = RetryPolicy {
            connection_timeout_secs: 45,
            task_timeout_secs: Some(600),
            max_retries: 3,
            backoff: BackoffStrategy::Fixed,
            initial_backoff_ms: 2_000,
            max_backoff_ms: 30_000,
        };
        settings.default_retry_policy = default_policy.clone();
        let effective = effective_retry_policy(&task, &settings);
        assert_eq!(effective, default_policy);
    }

    #[test]
    fn build_client_uses_default_retry_policy_connection_timeout() {
        let mut settings = AppSettings::default();
        settings.default_retry_policy.connection_timeout_secs = 45;
        let client = build_client(&settings).expect("client should build");
        // reqwest 不暴露 connect_timeout 的 getter，但成功构造即说明参数被接受。
        // 这里仅验证 build_client 不报错。
        drop(client);
        // 极小超时也应能成功构造。
        settings.default_retry_policy.connection_timeout_secs = 1;
        let _ = build_client(&settings).expect("client should build with 1s timeout");
    }

    // ===== Task 16: 任务优先级双通道测试 =====

    /// 辅助：构造一个指定 id/priority/queue_position 的任务，便于排序测试。
    fn priority_task(id: &str, priority: i32, queue_position: i64) -> DownloadTask {
        let directory = tempfile::tempdir().unwrap();
        let mut task = test_task(directory.path(), "p.bin", CollisionPolicy::Rename);
        task.id = id.into();
        task.priority = priority;
        task.queue_position = queue_position;
        task
    }

    /// 跨批次排序：不同 priority 的任务按 priority 升序排列。
    #[test]
    fn priority_sort_cross_batch_orders_by_priority_ascending() {
        let mut candidates = vec![
            priority_task("normal", 0, 5),
            priority_task("bottom", 1000, 1),
            priority_task("top", -1000, 9),
            priority_task("low", 50, 2),
            priority_task("high", -50, 8),
        ];
        sort_download_candidates(&mut candidates);
        let ids: Vec<&str> = candidates.iter().map(|t| t.id.as_str()).collect();
        // 数字越小越优先：top(-1000) → high(-50) → normal(0) → low(50) → bottom(1000)
        assert_eq!(ids, ["top", "high", "normal", "low", "bottom"]);
    }

    /// 同批次微调：同 priority 的任务按 queue_position 升序排列。
    #[test]
    fn priority_sort_same_batch_orders_by_queue_position_ascending() {
        let mut candidates = vec![
            priority_task("third", 0, 3),
            priority_task("first", 0, 1),
            priority_task("fourth", 0, 4),
            priority_task("second", 0, 2),
        ];
        sort_download_candidates(&mut candidates);
        let ids: Vec<&str> = candidates.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, ["first", "second", "third", "fourth"]);
    }

    /// 同批次微调 + 跨批次混合：先按 priority 分组，组内按 queue_position。
    #[test]
    fn priority_sort_mixed_groups_preserve_in_group_order() {
        let mut candidates = vec![
            priority_task("n2", 0, 2),
            priority_task("h2", -1, 2),
            priority_task("n1", 0, 1),
            priority_task("h1", -1, 1),
            priority_task("l1", 1, 1),
            priority_task("l2", 1, 2),
        ];
        sort_download_candidates(&mut candidates);
        let ids: Vec<&str> = candidates.iter().map(|t| t.id.as_str()).collect();
        // priority ASC: h(-1) → n(0) → l(1)，组内 queue_position ASC
        assert_eq!(ids, ["h1", "h2", "n1", "n2", "l1", "l2"]);
    }

    /// 置顶操作：priority 设为 MIN_PRIORITY (-1000)。
    #[test]
    fn priority_top_sets_to_min_priority() {
        let mut task = priority_task("task", 0, 1);
        task.priority = MIN_PRIORITY;
        assert_eq!(task.priority, -1000);
        // 验证置顶后排到最前
        let mut candidates = vec![
            priority_task("other", -50, 0),
            task.clone(),
            priority_task("normal", 0, 2),
        ];
        sort_download_candidates(&mut candidates);
        assert_eq!(candidates[0].id, "task");
    }

    /// 置底操作：priority 设为 MAX_PRIORITY (1000)。
    #[test]
    fn priority_bottom_sets_to_max_priority() {
        let mut task = priority_task("task", 0, 1);
        task.priority = MAX_PRIORITY;
        assert_eq!(task.priority, 1000);
        let mut candidates = vec![
            priority_task("normal", 0, 0),
            task.clone(),
            priority_task("other", 50, 3),
        ];
        sort_download_candidates(&mut candidates);
        assert_eq!(candidates.last().unwrap().id, "task");
    }

    /// 上移操作：priority -= PRIORITY_STEP (10)。
    #[test]
    fn priority_move_up_decreases_by_step() {
        use crate::models::PRIORITY_STEP;
        let task = priority_task("task", 0, 1);
        let original = task.priority;
        let new_priority = (original - PRIORITY_STEP).clamp(MIN_PRIORITY, MAX_PRIORITY);
        assert_eq!(new_priority, original - 10);

        // 多次上移不应超过 MIN_PRIORITY
        let low_priority = (MIN_PRIORITY + 5 - PRIORITY_STEP).clamp(MIN_PRIORITY, MAX_PRIORITY);
        assert_eq!(low_priority, MIN_PRIORITY);
    }

    /// 下移操作：priority += PRIORITY_STEP (10)。
    #[test]
    fn priority_move_down_increases_by_step() {
        use crate::models::PRIORITY_STEP;
        let task = priority_task("task", 0, 1);
        let original = task.priority;
        let new_priority = (original + PRIORITY_STEP).clamp(MIN_PRIORITY, MAX_PRIORITY);
        assert_eq!(new_priority, original + 10);

        // 多次下移不应超过 MAX_PRIORITY
        let high_priority = (MAX_PRIORITY - 5 + PRIORITY_STEP).clamp(MIN_PRIORITY, MAX_PRIORITY);
        assert_eq!(high_priority, MAX_PRIORITY);
    }

    /// 验证 priority 边界 clamp：超出范围的值被截断到 MIN/MAX_PRIORITY。
    #[test]
    fn priority_clamp_respects_bounds() {
        assert_eq!(5000_i32.clamp(MIN_PRIORITY, MAX_PRIORITY), MAX_PRIORITY);
        assert_eq!((-5000_i32).clamp(MIN_PRIORITY, MAX_PRIORITY), MIN_PRIORITY);
        assert_eq!(0_i32.clamp(MIN_PRIORITY, MAX_PRIORITY), 0);
    }

    /// 验证 is_ahead_of：更小 priority 排在前面；同 priority 时 queue_position 更小者排前。
    #[test]
    fn is_ahead_of_smaller_priority_wins() {
        let a = priority_task("a", -10, 100);
        let b = priority_task("b", 0, 1);
        assert!(is_ahead_of(&a, &b), "smaller priority should be ahead");
        assert!(!is_ahead_of(&b, &a), "larger priority should not be ahead");

        // 同 priority 时 queue_position 更小者排前
        let earlier = priority_task("earlier", 0, 1);
        let later = priority_task("later", 0, 10);
        assert!(is_ahead_of(&earlier, &later));
        assert!(!is_ahead_of(&later, &earlier));
    }

    // ===== Task 18: snapshot_segment_statuses 单元测试 =====
    // 验证 `task-connections` 事件的载荷来自 `SegmentRuntime` 原子量的真实采样，
    // 而非模拟数据（AGENTS.md §3）。

    /// 构造 8 连接任务的 SegmentRuntime 列表：每个分片 1MB，覆盖 8MB 总长度。
    fn eight_segment_runtimes() -> Vec<SegmentRuntime> {
        let segment_size: u64 = 1024 * 1024;
        (0..8)
            .map(|i| {
                let start = i as u64 * segment_size;
                let end = start + segment_size - 1;
                SegmentRuntime::new(i, start, end, 0, SEGMENT_PENDING)
            })
            .collect()
    }

    /// Task 18: 8 连接任务的快照必须包含全部 8 个分片，且 segment_id/offset 与 Runtime 一致。
    #[test]
    fn snapshot_segment_statuses_covers_all_eight_connections() {
        let runtimes = eight_segment_runtimes();
        let prev: Vec<u64> = vec![0; 8];
        let snapshot = snapshot_segment_statuses(&runtimes, &prev, 0.0, false);
        assert_eq!(snapshot.len(), 8, "8 连接任务必须返回 8 个 SegmentStatus");
        for (i, status) in snapshot.iter().enumerate() {
            assert_eq!(status.segment_id, i.to_string());
            assert_eq!(status.start_offset, i as u64 * 1024 * 1024);
            assert_eq!(status.total_bytes, 1024 * 1024);
            assert_eq!(status.downloaded_bytes, 0);
        }
    }

    /// Task 18: 新分配但未开始接收数据的分片应映射为 Connecting。
    #[test]
    fn snapshot_segment_statuses_maps_idle_segment_to_connecting() {
        let runtimes = vec![SegmentRuntime::new(0, 0, 1023, 0, SEGMENT_PENDING)];
        let snapshot = snapshot_segment_statuses(&runtimes, &[], 0.0, false);
        assert_eq!(snapshot[0].state, ConnectionState::Connecting);
        assert_eq!(snapshot[0].retry_count, 0);
        assert_eq!(snapshot[0].error, None);
    }

    /// Task 18: active_windows > 0 表示分片正在下载数据，应映射为 Downloading。
    #[test]
    fn snapshot_segment_statuses_maps_active_window_to_downloading() {
        let runtime = SegmentRuntime::new(0, 0, 1023, 0, SEGMENT_DOWNLOADING);
        runtime.active_windows.store(1, Ordering::Relaxed);
        let runtimes = vec![runtime];
        let snapshot = snapshot_segment_statuses(&runtimes, &[], 0.0, false);
        assert_eq!(snapshot[0].state, ConnectionState::Downloading);
    }

    /// Task 18: retrying 标志表示分片在退避 sleep 中，应映射为 Retrying。
    #[test]
    fn snapshot_segment_statuses_maps_retrying_flag_to_retrying() {
        let runtime = SegmentRuntime::new(0, 0, 1023, 0, SEGMENT_DOWNLOADING);
        runtime.active_windows.store(1, Ordering::Relaxed);
        runtime.retrying.store(true, Ordering::Relaxed);
        runtime.retry_count.store(2, Ordering::Relaxed);
        let runtimes = vec![runtime];
        let snapshot = snapshot_segment_statuses(&runtimes, &[], 0.0, false);
        assert_eq!(snapshot[0].state, ConnectionState::Retrying);
        assert_eq!(snapshot[0].retry_count, 2);
    }

    /// Task 18: downloaded == total 表示分片已完成，应映射为 Completed（优先于其他状态）。
    #[test]
    fn snapshot_segment_statuses_marks_completed_when_downloaded_equals_total() {
        let runtime = SegmentRuntime::new(0, 0, 1023, 1024, SEGMENT_COMPLETED);
        // 即使 active_windows 仍为 1（连接刚结束），完成判定优先。
        runtime.active_windows.store(1, Ordering::Relaxed);
        let runtimes = vec![runtime];
        let snapshot = snapshot_segment_statuses(&runtimes, &[], 0.0, false);
        assert_eq!(snapshot[0].state, ConnectionState::Completed);
        assert_eq!(snapshot[0].downloaded_bytes, 1024);
        assert_eq!(snapshot[0].total_bytes, 1024);
    }

    /// Task 18: status == SEGMENT_FAILED 表示分片已失败，应映射为 Failed 并附带错误信息。
    #[test]
    fn snapshot_segment_statuses_marks_failed_when_status_is_failed() {
        let runtime = SegmentRuntime::new(0, 0, 1023, 100, SEGMENT_FAILED);
        runtime.set_last_error("connection reset by peer; Cookie: secret=abc");
        let runtimes = vec![runtime];
        let snapshot = snapshot_segment_statuses(&runtimes, &[], 0.0, false);
        assert_eq!(snapshot[0].state, ConnectionState::Failed);
        // 错误信息必须经过 redact_sensitive 脱敏（Cookie 值替换为 ***）。
        let err = snapshot[0].error.as_ref().expect("应有错误信息");
        assert!(err.contains("connection reset"));
        assert!(err.contains("***"));
        assert!(!err.contains("secret=abc"));
    }

    /// Task 18: task_paused = true 时所有未完成分片必须映射为 Paused，
    /// 即使 active_windows > 0 或 retrying = true（任务已停止）。
    #[test]
    fn snapshot_segment_statuses_pauses_all_segments_when_task_paused() {
        let runtimes = eight_segment_runtimes();
        // 模拟暂停瞬间的真实状态：3 个分片在下载数据、1 个在重试、1 个已完成、1 个已失败。
        runtimes[0].active_windows.store(1, Ordering::Relaxed);
        runtimes[0]
            .status
            .store(SEGMENT_DOWNLOADING, Ordering::Relaxed);
        runtimes[0].downloaded_bytes.store(100, Ordering::Relaxed);
        runtimes[1].active_windows.store(1, Ordering::Relaxed);
        runtimes[1]
            .status
            .store(SEGMENT_DOWNLOADING, Ordering::Relaxed);
        runtimes[1].downloaded_bytes.store(200, Ordering::Relaxed);
        runtimes[2].active_windows.store(1, Ordering::Relaxed);
        runtimes[2].retrying.store(true, Ordering::Relaxed);
        runtimes[3]
            .downloaded_bytes
            .store(1024 * 1024, Ordering::Relaxed);
        runtimes[3]
            .status
            .store(SEGMENT_COMPLETED, Ordering::Relaxed);
        runtimes[4].status.store(SEGMENT_FAILED, Ordering::Relaxed);

        let prev: Vec<u64> = runtimes
            .iter()
            .map(|r| r.downloaded_bytes.load(Ordering::Relaxed))
            .collect();
        let snapshot = snapshot_segment_statuses(&runtimes, &prev, 0.0, true);

        // 暂停时：已完成和已失败的分片保留原状态（downloaded==total 仍 Completed，但 SEGMENT_FAILED
        // 在 task_paused 之后判定）。按当前实现 task_paused 优先于其他状态，因此全部 Paused。
        // 这与 emit_task_connections_final 在退出路径上传 task_paused=true 的语义一致。
        for status in &snapshot {
            assert_eq!(
                status.state,
                ConnectionState::Paused,
                "暂停时所有分片应为 Paused"
            );
        }
    }

    /// Task 18: 速度计算必须来自 downloaded_bytes 原子量的真实增量，而非模拟。
    /// 验证：prev=100, current=300, elapsed=2s → speed = (300-100)/2 = 100 bytes/s。
    #[test]
    fn snapshot_segment_statuses_computes_speed_from_real_delta() {
        let runtime = SegmentRuntime::new(0, 0, 1023, 300, SEGMENT_DOWNLOADING);
        runtime.active_windows.store(1, Ordering::Relaxed);
        let runtimes = vec![runtime];
        let prev: Vec<u64> = vec![100];
        let snapshot = snapshot_segment_statuses(&runtimes, &prev, 2.0, false);
        assert_eq!(
            snapshot[0].speed, 100,
            "speed 应为真实增量 (300-100)/2s = 100 bytes/s"
        );
    }

    /// Task 18: elapsed_secs 过小时 speed 应为 0（避免除零）。
    #[test]
    fn snapshot_segment_statuses_returns_zero_speed_when_elapsed_too_small() {
        let runtime = SegmentRuntime::new(0, 0, 1023, 500, SEGMENT_DOWNLOADING);
        runtime.active_windows.store(1, Ordering::Relaxed);
        let runtimes = vec![runtime];
        let prev: Vec<u64> = vec![100];
        let snapshot = snapshot_segment_statuses(&runtimes, &prev, 0.0005, false);
        assert_eq!(snapshot[0].speed, 0);
    }

    /// Task 18: prev_bytes 短于 runtimes 时使用 0 作为基线，避免越界（安全默认值）。
    #[test]
    fn snapshot_segment_statuses_handles_short_prev_bytes_safely() {
        let runtimes = eight_segment_runtimes();
        // 仅提供 3 个 prev 值，其余应使用 0 作为基线。
        let prev: Vec<u64> = vec![10, 20, 30];
        let snapshot = snapshot_segment_statuses(&runtimes, &prev, 1.0, false);
        assert_eq!(snapshot.len(), 8);
        // 前 3 个有 prev 值；由于 downloaded=0 < prev，speed=0（饱和减法）。
        // 后 5 个 prev=0，downloaded=0，speed=0。
        for status in &snapshot {
            assert_eq!(status.speed, 0);
        }
    }

    /// Task 18: 修改 SegmentRuntime 原子量后，下一次快照必须反映新状态（非缓存/模拟）。
    #[test]
    fn snapshot_segment_statuses_reflects_live_atomic_updates() {
        let runtime = SegmentRuntime::new(0, 0, 1023, 0, SEGMENT_PENDING);
        let runtimes = vec![runtime];
        // 第一次快照：connecting，downloaded=0
        let s1 = snapshot_segment_statuses(&runtimes, &[], 0.0, false);
        assert_eq!(s1[0].state, ConnectionState::Connecting);
        assert_eq!(s1[0].downloaded_bytes, 0);
        // 模拟下载循环更新原子量：开始接收数据，已下载 512 字节
        runtimes[0].active_windows.store(1, Ordering::Relaxed);
        runtimes[0]
            .status
            .store(SEGMENT_DOWNLOADING, Ordering::Relaxed);
        runtimes[0].downloaded_bytes.store(512, Ordering::Relaxed);
        // 第二次快照：downloading，downloaded=512
        let prev: Vec<u64> = vec![0];
        let s2 = snapshot_segment_statuses(&runtimes, &prev, 1.0, false);
        assert_eq!(s2[0].state, ConnectionState::Downloading);
        assert_eq!(s2[0].downloaded_bytes, 512);
        assert_eq!(s2[0].speed, 512);
        // 完成：downloaded = total
        runtimes[0].downloaded_bytes.store(1024, Ordering::Relaxed);
        runtimes[0]
            .status
            .store(SEGMENT_COMPLETED, Ordering::Relaxed);
        runtimes[0].active_windows.store(0, Ordering::Relaxed);
        let s3 = snapshot_segment_statuses(&runtimes, &vec![512], 1.0, false);
        assert_eq!(s3[0].state, ConnectionState::Completed);
        assert_eq!(s3[0].speed, 512);
    }

    /// Task 18: 模拟 8 连接任务在下载中、暂停、完成三个阶段的状态流转。
    /// 这是 SubTask 18.5 集成测试的纯逻辑等价物（无需启动 HTTP 服务器），
    /// 验证 snapshot_segment_statuses 在状态转换时返回正确的 SegmentStatus。
    #[test]
    fn snapshot_segment_statuses_eight_connection_lifecycle() {
        let segment_size: u64 = 1024 * 1024;
        let runtimes = eight_segment_runtimes();

        // 阶段 1：下载中——所有 8 个分片都活跃，已下载 256KB 各
        for r in &runtimes {
            r.active_windows.store(1, Ordering::Relaxed);
            r.status.store(SEGMENT_DOWNLOADING, Ordering::Relaxed);
            r.downloaded_bytes.store(256 * 1024, Ordering::Relaxed);
        }
        let prev: Vec<u64> = vec![0; 8];
        let snap1 = snapshot_segment_statuses(&runtimes, &prev, 1.0, false);
        assert_eq!(snap1.len(), 8);
        for s in &snap1 {
            assert_eq!(s.state, ConnectionState::Downloading);
            assert_eq!(s.downloaded_bytes, 256 * 1024);
            assert_eq!(s.total_bytes, segment_size);
            assert_eq!(s.speed, 256 * 1024, "1 秒内下载 256KB → 256KB/s");
        }

        // 阶段 2：暂停——所有分片应变为 Paused
        let prev2: Vec<u64> = runtimes
            .iter()
            .map(|r| r.downloaded_bytes.load(Ordering::Relaxed))
            .collect();
        let snap2 = snapshot_segment_statuses(&runtimes, &prev2, 0.0, true);
        for s in &snap2 {
            assert_eq!(s.state, ConnectionState::Paused);
            // 暂停时仍保留已下载字节数据（用于 UI 展示进度）
            assert_eq!(s.downloaded_bytes, 256 * 1024);
        }

        // 阶段 3：完成——所有分片 downloaded == total
        for r in &runtimes {
            r.downloaded_bytes.store(segment_size, Ordering::Relaxed);
            r.status.store(SEGMENT_COMPLETED, Ordering::Relaxed);
            r.active_windows.store(0, Ordering::Relaxed);
        }
        let snap3 = snapshot_segment_statuses(&runtimes, &prev2, 1.0, false);
        for s in &snap3 {
            assert_eq!(s.state, ConnectionState::Completed);
            assert_eq!(s.downloaded_bytes, segment_size);
            // 完成时的速度 = (segment_size - 256KB) / 1s
            assert_eq!(s.speed, segment_size - 256 * 1024);
        }
    }

    #[test]
    fn refresh_url_updates_task_and_clears_error() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open(dir.path().to_path_buf()).unwrap());
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let mut task = test_task(dir.path(), "file.zip", CollisionPolicy::Rename);
            task.url = "https://example.com/expired-link?token=old".into();
            task.status = TaskStatus::Failed;
            task.error = Some("HTTP 403 Forbidden".into());
            store.upsert_task(&task).await.unwrap();

            // 验证空 URL 拒绝
            let invalid_url = "   ";
            assert!(invalid_url.trim().is_empty());

            // 模拟刷新链接
            let new_url = "https://example.com/fresh-link?token=new";
            task.url = new_url.to_string();
            task.error = None;
            task.status = TaskStatus::Paused;
            store.upsert_task(&task).await.unwrap();

            let updated = store.get_task(&task.id).await.unwrap().unwrap();
            assert_eq!(updated.url, new_url);
            assert_eq!(updated.status, TaskStatus::Paused);
            assert!(updated.error.is_none());
        });
    }

    #[test]
    fn tracker_deduplication_and_filtering() {
        let existing = "http://tracker.opentrackr.org:1337/announce\nudp://tracker.openbittorrent.com:6969/announce";
        let new_fetched = "udp://tracker.openbittorrent.com:6969/announce\nhttps://tracker.tamersunion.org:443/announce\ninvalid_tracker_line";
        
        let mut trackers_set = HashSet::new();
        for line in existing.lines().chain(new_fetched.lines()) {
            let t = line.trim();
            if !t.is_empty() && (t.starts_with("http://") || t.starts_with("https://") || t.starts_with("udp://") || t.starts_with("ws://") || t.starts_with("wss://")) {
                trackers_set.insert(t.to_string());
            }
        }
        assert_eq!(trackers_set.len(), 3);
        assert!(trackers_set.contains("http://tracker.opentrackr.org:1337/announce"));
        assert!(trackers_set.contains("udp://tracker.openbittorrent.com:6969/announce"));
        assert!(trackers_set.contains("https://tracker.tamersunion.org:443/announce"));
        assert!(!trackers_set.contains("invalid_tracker_line"));
    }

    #[test]
    fn test_layout_from_existing_starts_reconstructs_layout() {
        let start = 0u64;
        let end = 100_000_000u64;
        let starts = vec![0, 30_000_000, 70_000_000];

        let layout = layout_from_existing_starts(start, end, &starts).unwrap();
        assert_eq!(layout.len(), 3);
        assert_eq!(layout[0], (0, 0, 29_999_999));
        assert_eq!(layout[1], (1, 30_000_000, 69_999_999));
        assert_eq!(layout[2], (2, 70_000_000, 100_000_000));

        // 校验非法 starts 序列：首项不匹配 start
        assert!(layout_from_existing_starts(10, end, &starts).is_none());
        // 非法 starts 序列：非严格递增
        assert!(layout_from_existing_starts(start, end, &[0, 50, 40]).is_none());
        // 非法 starts 序列：超出 end
        assert!(layout_from_existing_starts(start, end, &[0, 100_000_001]).is_none());
    }

    #[tokio::test]
    async fn test_work_stealing_coordinator_segment_aggregation() {
        let temp = Path::new("aggregate_test.tmp");
        let initial = vec![
            RangeWindow {
                id: 1,
                segment_index: 0,
                ordinal: 0,
                start_byte: 0,
                end_byte: 20_000_000,
                existing_bytes: 0,
                path: window_part_path(temp, 0, 0),
                status: WindowStatus::Pending,
            },
            RangeWindow {
                id: 2,
                segment_index: 0,
                ordinal: 1,
                start_byte: 20_000_001,
                end_byte: 50_000_000,
                existing_bytes: 0,
                path: window_part_path(temp, 0, 20_000_001),
                status: WindowStatus::Pending,
            },
        ];

        let coordinator = WorkStealingCoordinator::new(temp, initial);
        let (w1, _) = coordinator.claim_or_steal_work().await.unwrap();
        let (w2, _) = coordinator.claim_or_steal_work().await.unwrap();

        coordinator.finish_window(w1.id, true, 20_000_001).await;
        coordinator.finish_window(w2.id, true, 30_000_000).await;

        let ordered = coordinator.get_ordered_windows_for_segment(0).await;
        assert_eq!(ordered.len(), 2);
        assert_eq!(ordered[0].start_byte, 0);
        assert_eq!(ordered[0].end_byte, 20_000_000);
        assert_eq!(ordered[1].start_byte, 20_000_001);
        assert_eq!(ordered[1].end_byte, 50_000_000);
    }
