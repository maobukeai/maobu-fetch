use super::*;

struct Fixture {
    _root: tempfile::TempDir,
    temp: PathBuf,
    merge: PathBuf,
    parts: Vec<(PathBuf, Vec<u8>)>,
    expected: Vec<u8>,
    plan: plan::Plan,
}
impl Fixture {
    async fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let temp = root.path().join("download.lumaget");
        let merge = root.path().join("download.lumaget.merge");
        let expected: Vec<u8> = (0..262_144).map(|i| ((i * 73 + 19) % 251) as u8).collect();
        fs::write(&temp, b"existing validated temporary output")
            .await
            .unwrap();
        let prefix = PathBuf::from(format!("{}.part0", temp.display()));
        fs::write(&prefix, &expected[..7]).await.unwrap();
        let mut parts = vec![(prefix, expected[..7].to_vec())];
        let mut windows = Vec::new();
        for (ordinal, (start, end)) in [(7u64, 100_000u64), (100_001, 262_143)]
            .into_iter()
            .enumerate()
        {
            let path = window_part_path(&temp, 0, start);
            let bytes = expected[start as usize..=end as usize].to_vec();
            fs::write(&path, &bytes).await.unwrap();
            parts.push((path.clone(), bytes));
            windows.push(RangeWindow {
                id: ordinal as u64 + 1,
                segment_index: 0,
                ordinal: ordinal as u32,
                start_byte: start,
                end_byte: end,
                existing_bytes: end - start + 1,
                path,
                status: WindowStatus::Completed,
            });
        }
        let plan = plan::Plan {
            ranges: vec![(0, 0, expected.len() as u64 - 1)],
            initial: expected.len() as u64,
            coordinator: Arc::new(WorkStealingCoordinator::new(&temp, windows)),
            runtimes: Arc::new(Vec::new()),
        };
        Self {
            _root: root,
            temp,
            merge,
            parts,
            expected,
            plan,
        }
    }
    async fn assert_parts(&self) {
        for (path, bytes) in &self.parts {
            assert_eq!(fs::read(path).await.unwrap(), *bytes);
        }
    }
}

#[tokio::test]
async fn actual_merge_keeps_all_source_parts_and_atomically_replaces_only_complete_bytes() {
    let fixture = Fixture::new().await;
    let token = CancellationToken::new();
    let input = MergeInput {
        plan: &fixture.plan,
        total: fixture.expected.len() as u64,
        token: &token,
    };
    write_merge(&fixture.merge, &fixture.temp, &input, 4096)
        .await
        .unwrap();
    let actual = fs::read(&fixture.temp).await.unwrap();
    assert_eq!(actual.len(), fixture.expected.len());
    assert_eq!(Sha256::digest(&actual), Sha256::digest(&fixture.expected));
    assert!(!fixture.merge.exists());
    fixture.assert_parts().await;
}

#[tokio::test]
async fn malformed_or_incomplete_merge_never_replaces_the_old_output_or_deletes_parts() {
    for kind in 0..3 {
        let mut fixture = Fixture::new().await;
        let total = fixture.expected.len() as u64;
        if kind == 0 {
            let (path, bytes) = &mut fixture.parts[2];
            bytes.pop();
            fs::write(path, bytes).await.unwrap();
        } else if kind == 1 {
            // An absent exact-start window must remain an integrity error rather than closing the gap.
            fixture.plan.coordinator =
                Arc::new(WorkStealingCoordinator::new(&fixture.temp, Vec::new()));
        }
        let token = CancellationToken::new();
        let input = MergeInput {
            plan: &fixture.plan,
            total: total + u64::from(kind == 2),
            token: &token,
        };
        assert!(write_merge(&fixture.merge, &fixture.temp, &input, 4096)
            .await
            .is_err());
        assert_eq!(
            fs::read(&fixture.temp).await.unwrap(),
            b"existing validated temporary output"
        );
        fixture.assert_parts().await;
    }
}

#[tokio::test]
async fn cancellation_during_actual_merge_keeps_the_old_output_and_source_bytes() {
    let root = tempfile::tempdir().unwrap();
    let temp = root.path().join("record.lumaget");
    let merge = root.path().join("record.lumaget.merge");
    let part = PathBuf::from(format!("{}.part0", temp.display()));
    fs::write(&temp, b"old-complete-output").await.unwrap();
    let source = fs::File::create(&part).await.unwrap();
    let total = 64 * 1024 * 1024;
    source.set_len(total).await.unwrap();
    drop(source);
    let plan = plan::Plan {
        ranges: vec![(0, 0, total - 1)],
        initial: total,
        coordinator: Arc::new(WorkStealingCoordinator::new(&temp, Vec::new())),
        runtimes: Arc::new(Vec::new()),
    };
    let token = CancellationToken::new();
    let stopper = token.clone();
    let watch = merge.clone();
    let watcher = tokio::spawn(async move {
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if fs::metadata(&watch)
                    .await
                    .is_ok_and(|metadata| metadata.len() > 0)
                {
                    stopper.cancel();
                    break;
                }
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .unwrap();
    });
    let input = MergeInput {
        plan: &plan,
        total,
        token: &token,
    };
    let error = write_merge(&merge, &temp, &input, 4096).await.unwrap_err();
    watcher.await.unwrap();
    assert!(error.contains("暂停"));
    assert_eq!(fs::read(&temp).await.unwrap(), b"old-complete-output");
    assert_eq!(fs::metadata(&part).await.unwrap().len(), total);
    assert!(fs::metadata(&merge).await.unwrap().len() < total);
    let mut source = fs::File::open(&part).await.unwrap();
    let mut bytes = [1u8; 4096];
    source.read_exact(&mut bytes).await.unwrap();
    assert_eq!(bytes, [0; 4096]);
}
