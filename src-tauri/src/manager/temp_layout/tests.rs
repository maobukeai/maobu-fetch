use super::*;
#[tokio::test]
async fn output_name_changes_do_not_orphan_existing_recovery_data() {
    let directory = tempfile::tempdir().unwrap();
    let mut task = crate::bt::tests::bt_task_for_test();
    task.destination = directory.path().to_string_lossy().into_owned();
    task.file_name = "video.mp4".into();
    let original = ensure(&task).await.unwrap();
    fs::write(&original, b"resume bytes").await.unwrap();
    task.file_name = "video (1).ts".into();
    assert_eq!(resolve(&task).await.unwrap(), original);
    assert_eq!(ensure(&task).await.unwrap(), original);
    assert_eq!(fs::read(original).await.unwrap(), b"resume bytes");
    let invalid = Layout {
        version: 1,
        task_id: task.id.clone(),
        source_name: "../outside".into(),
        legacy_root: false,
    };
    fs::write(
        task_temp_dir(&task.destination, &task.id).join(LAYOUT),
        serde_json::to_vec(&invalid).unwrap(),
    )
    .await
    .unwrap();
    assert!(resolve(&task).await.is_err());
}

async fn owned_fixture() -> (tempfile::TempDir, Store, DownloadTask) {
    let directory = tempfile::tempdir().unwrap();
    let data = directory.path().join("data");
    let store = tokio::task::spawn_blocking(move || Store::open(data))
        .await
        .unwrap()
        .unwrap();
    let mut task = crate::bt::tests::bt_task_for_test();
    task.task_kind = TaskKind::Http;
    task.destination = directory.path().to_string_lossy().into_owned();
    task.file_name = "legacy.bin".into();
    store.upsert_task(&task).await.unwrap();
    (directory, store, task)
}

#[tokio::test]
async fn unique_old_layout_is_reused_and_cleared_by_its_original_name() {
    let (_root, store, mut task) = owned_fixture().await;
    let old = PathBuf::from(&task.destination).join("legacy.bin.lumaget");
    let shard = PathBuf::from(format!("{}.part0", old.to_string_lossy()));
    fs::write(&shard, b"old verified prefix").await.unwrap();
    assert_eq!(ensure_owned(&task, &store).await.unwrap(), old);
    task.file_name = "renamed.bin".into();
    store.upsert_task(&task).await.unwrap();
    assert_eq!(ensure_owned(&task, &store).await.unwrap(), old);
    assert_eq!(fs::read(&shard).await.unwrap(), b"old verified prefix");
    clear_task_parts(&task, &store).await.unwrap();
    assert!(!shard.exists());
}

#[tokio::test]
async fn a_new_task_neither_borrows_nor_deletes_flat_files_with_the_same_name() {
    let (_root, store, task) = owned_fixture().await;
    let legacy = PathBuf::from(&task.destination).join("legacy.bin.lumaget.part0");
    fs::write(&legacy, b"another old task").await.unwrap();
    store.remove_task(&task.id).await.unwrap();
    store
        .create_task_with_name_intent(&task, false)
        .await
        .unwrap();
    let source = ensure_owned(&task, &store).await.unwrap();
    assert_eq!(
        source,
        task_temp_path(&task.destination, &task.id, &task.file_name)
    );
    assert!(!startup_recovery::has_existing_bytes(&task, &store)
        .await
        .unwrap());
    fs::write(&source, b"new payload").await.unwrap();
    assert!(startup_recovery::has_existing_bytes(&task, &store)
        .await
        .unwrap());
    clear_task_parts(&task, &store).await.unwrap();
    assert_eq!(fs::read(legacy).await.unwrap(), b"another old task");
}

#[tokio::test]
async fn ambiguous_old_names_refuse_adoption_and_preserve_all_files() {
    let (_root, store, task) = owned_fixture().await;
    let legacy = PathBuf::from(&task.destination).join("legacy.bin.lumaget");
    fs::write(&legacy, b"ambiguous old bytes").await.unwrap();
    let mut other = task.clone();
    other.id = Uuid::new_v4().to_string();
    store.upsert_task(&other).await.unwrap();
    assert!(ensure_owned(&task, &store)
        .await
        .unwrap_err()
        .contains("多个"));
    assert!(clear_task_parts(&task, &store).await.is_err());
    assert_eq!(fs::read(legacy).await.unwrap(), b"ambiguous old bytes");
}

#[tokio::test]
async fn disk_prefix_is_detected_when_sqlite_still_records_zero() {
    let (_root, store, mut task) = owned_fixture().await;
    task.downloaded_bytes = 0;
    task.segments.clear();
    assert!(!startup_recovery::has_existing_bytes(&task, &store)
        .await
        .unwrap());
    let source = ensure_owned(&task, &store).await.unwrap();
    fs::write(
        format!("{}.part0.w0", source.to_string_lossy()),
        b"durable prefix",
    )
    .await
    .unwrap();
    assert!(startup_recovery::has_existing_bytes(&task, &store)
        .await
        .unwrap());
    assert_eq!(
        fs::read(format!("{}.part0.w0", source.to_string_lossy()))
            .await
            .unwrap(),
        b"durable prefix"
    );
}

#[tokio::test]
async fn renamed_legacy_owner_still_blocks_another_old_task_from_claiming_its_source() {
    let (_root, store, mut owner) = owned_fixture().await;
    let original_name = owner.file_name.clone();
    let source = PathBuf::from(&owner.destination).join(format!("{original_name}.lumaget"));
    fs::write(&source, b"legacy owner durable payload")
        .await
        .unwrap();
    assert_eq!(ensure_owned(&owner, &store).await.unwrap(), source);
    owner.file_name = "final-renamed-output.bin".into();
    store.upsert_task(&owner).await.unwrap();
    assert_eq!(resolve(&owner).await.unwrap(), source);

    let mut other = owner.clone();
    other.id = Uuid::new_v4().to_string();
    other.file_name = original_name;
    store.upsert_task(&other).await.unwrap();
    assert!(ensure_owned(&other, &store)
        .await
        .unwrap_err()
        .contains("多个"));
    assert!(clear_task_parts(&other, &store).await.is_err());
    assert_eq!(
        fs::read(&source).await.unwrap(),
        b"legacy owner durable payload"
    );
}

#[cfg(windows)]
#[tokio::test]
async fn actual_junction_with_matching_markers_is_rejected_by_resolve_resume_and_recovery() {
    let (root, store, mut task) = owned_fixture().await;
    task.status = TaskStatus::Downloading;
    store.upsert_task(&task).await.unwrap();
    let private_source = ensure_owned(&task, &store).await.unwrap();
    let private = task_temp_dir(&task.destination, &task.id);
    let layout_bytes = fs::read(private.join(LAYOUT)).await.unwrap();
    let foreign = root.path().join("foreign-user-files");
    fs::create_dir(&foreign).await.unwrap();
    fs::write(foreign.join(LAYOUT), &layout_bytes)
        .await
        .unwrap();
    let foreign_source = foreign.join(private_source.file_name().unwrap());
    fs::write(
        &foreign_source,
        b"foreign payload must not be appended or removed",
    )
    .await
    .unwrap();
    let sentinel = foreign.join("unrelated-sentinel.bin");
    fs::write(&sentinel, b"unrelated foreign data")
        .await
        .unwrap();
    let published = PathBuf::from(&task.destination).join(&task.file_name);
    fs::write(&published, b"already published fixture")
        .await
        .unwrap();
    let journal = serde_json::json!({"version":1,"task_id":task.id,"file_name":task.file_name,
        "length":b"already published fixture".len(),"sha256":hex::encode(Sha256::digest(b"already published fixture"))});
    let journal_bytes = serde_json::to_vec(&journal).unwrap();
    fs::write(foreign.join("task-publication.json"), &journal_bytes)
        .await
        .unwrap();

    // Only remove the empty owned directory, never recurse through a computed junction path.
    fs::remove_file(private.join(LAYOUT)).await.unwrap();
    fs::remove_dir(&private).await.unwrap();
    assert!(private.starts_with(root.path()) && foreign.starts_with(root.path()));
    create_actual_junction(&private, &foreign).await;

    assert!(resolve(&task).await.is_err());
    assert!(ensure_owned(&task, &store).await.is_err());
    assert!(startup_recovery::has_existing_bytes(&task, &store)
        .await
        .is_err());
    assert!(
        super::super::task_publication::recover(&task, &CancellationToken::new())
            .await
            .is_err()
    );
    execute_selfcheck(&store).await.unwrap();
    assert_eq!(
        store.get_task(&task.id).await.unwrap().unwrap().status,
        TaskStatus::Failed
    );
    assert!(clear_task_parts(&task, &store).await.is_err());
    assert_eq!(
        fs::read(&foreign_source).await.unwrap(),
        b"foreign payload must not be appended or removed"
    );
    assert_eq!(
        fs::read(&sentinel).await.unwrap(),
        b"unrelated foreign data"
    );
    assert_eq!(fs::read(foreign.join(LAYOUT)).await.unwrap(), layout_bytes);
    assert_eq!(
        fs::read(foreign.join("task-publication.json"))
            .await
            .unwrap(),
        journal_bytes
    );
    assert_eq!(
        fs::read(&published).await.unwrap(),
        b"already published fixture"
    );
    // RemoveDirectory removes the junction itself; verify its independent target survives.
    fs::remove_dir(&private).await.unwrap();
    assert_eq!(fs::read(sentinel).await.unwrap(), b"unrelated foreign data");
}

#[cfg(windows)]
#[tokio::test]
async fn two_legacy_download_directories_aliasing_the_same_target_are_not_unique_owners() {
    let (root, store, mut task) = owned_fixture().await;
    let target = root.path().join("actual-download-root");
    let alias = root.path().join("aliased-download-root");
    fs::create_dir(&target).await.unwrap();
    assert!(target.starts_with(root.path()) && alias.starts_with(root.path()));
    create_actual_junction(&alias, &target).await;
    task.destination = target.to_string_lossy().into_owned();
    store.upsert_task(&task).await.unwrap();
    let mut other = task.clone();
    other.id = Uuid::new_v4().to_string();
    other.destination = alias.to_string_lossy().into_owned();
    store.upsert_task(&other).await.unwrap();
    let shard = target.join(format!("{}.lumaget.part0", task.file_name));
    fs::write(&shard, b"ambiguous shared physical shard")
        .await
        .unwrap();
    assert!(ensure_owned(&task, &store)
        .await
        .unwrap_err()
        .contains("多个"));
    assert!(ensure_owned(&other, &store)
        .await
        .unwrap_err()
        .contains("多个"));
    assert!(clear_task_parts(&task, &store).await.is_err());
    assert!(clear_task_parts(&other, &store).await.is_err());
    assert_eq!(
        fs::read(&shard).await.unwrap(),
        b"ambiguous shared physical shard"
    );
    fs::remove_dir(&alias).await.unwrap();
    assert_eq!(
        fs::read(shard).await.unwrap(),
        b"ambiguous shared physical shard"
    );
}

#[cfg(windows)]
async fn create_actual_junction(link: &Path, target: &Path) {
    let mut command = crate::media_tools::create_hidden_tokio_command("cmd.exe");
    command
        .kill_on_drop(true)
        .args(["/D", "/C", "mklink", "/J"])
        .arg(link)
        .arg(target);
    let result = tokio::time::timeout(std::time::Duration::from_secs(5), command.output())
        .await
        .unwrap()
        .unwrap();
    assert!(
        result.status.success(),
        "actual Windows junction fixture must be created"
    );
    assert!(redirected(&fs::symlink_metadata(link).await.unwrap()));
    assert_eq!(
        fs::canonicalize(link).await.unwrap(),
        fs::canonicalize(target).await.unwrap()
    );
}
