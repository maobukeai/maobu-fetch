use super::*;
use rusqlite::params;

fn old_database(path: &std::path::Path) -> Connection {
    std::fs::create_dir_all(path).unwrap();
    let connection = Connection::open(path.join("lumaget.db")).unwrap();
    connection.execute_batch("CREATE TABLE platform_compatibility(platform TEXT PRIMARY KEY, level TEXT NOT NULL, notes TEXT NOT NULL, known_issues_json TEXT NOT NULL DEFAULT '[]', last_tested_at TEXT NOT NULL DEFAULT '');").unwrap();
    connection
}

fn evidence() -> PlatformVerificationEvidence {
    PlatformVerificationEvidence {
        app_version: env!("CARGO_PKG_VERSION").into(),
        tool_version: "fixture-tool 1.0".into(),
        sample: "local-fixture/sample-1".into(),
        scope: "download".into(),
        result: "passed".into(),
    }
}

fn insert_evidence(
    store: &Store,
    platform: &str,
    date: &str,
    value: &PlatformVerificationEvidence,
) {
    store.connection.blocking_lock().execute("INSERT OR REPLACE INTO platform_compatibility(platform,level,notes,known_issues_json,last_tested_at,verification_json) VALUES(?1,'verified','用户说明','[]',?2,?3)",
        params![platform,date,serde_json::to_string(value).unwrap()]).unwrap();
}

#[test]
fn legacy_defaults_downgrade_without_overwriting_custom_rows_after_restart() {
    let directory = tempfile::tempdir().unwrap();
    {
        let connection = old_database(directory.path());
        connection.execute("INSERT INTO platform_compatibility(platform,level,notes) VALUES('bilibili','verified','哔哩哔哩普通视频、多P合集、番剧剧集与实时直播流均可正常下载')", []).unwrap();
        for platform in ["lanzou", "pan123"] {
            connection.execute("INSERT INTO platform_compatibility(platform,level,notes,known_issues_json) VALUES(?1,'unsupported','用户自定义说明','[\"保留自定义问题\"]')", [platform]).unwrap();
        }
        connection.execute("INSERT INTO platform_compatibility(platform,level,notes) VALUES('custom-site','verified','用户自己的原始声明')", []).unwrap();
    }
    for _ in 0..3 {
        let store = Store::open(directory.path().to_owned()).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let bili = store
                .platform_compatibility_get("bilibili")
                .await
                .unwrap()
                .unwrap();
            assert_eq!(bili.level, SupportLevel::Experimental);
            assert!(bili.last_tested_at.is_empty() && bili.verification.is_none());
            assert!(!bili.notes.contains("均可正常下载"));
            for platform in ["lanzou", "pan123"] {
                let item = store
                    .platform_compatibility_get(platform)
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(item.level, SupportLevel::Unsupported);
                assert_eq!(item.notes, "用户自定义说明");
                assert_eq!(item.known_issues, vec!["保留自定义问题"]);
            }
            let custom = store
                .platform_compatibility_get("custom-site")
                .await
                .unwrap()
                .unwrap();
            assert_eq!(custom.level, SupportLevel::Experimental);
            assert_eq!(custom.notes, "用户自己的原始声明");
        });
        let raw: String = store
            .connection
            .blocking_lock()
            .query_row(
                "SELECT level FROM platform_compatibility WHERE platform='custom-site'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            raw, "verified",
            "原始用户声明保留在数据库，展示等级由证据约束"
        );
    }
}

#[test]
fn verified_requires_current_version_complete_download_evidence_and_round_trips() {
    let directory = tempfile::tempdir().unwrap();
    let valid = evidence();
    {
        let store = Store::open(directory.path().to_owned()).unwrap();
        insert_evidence(&store, "custom-verified", "1970-01-01T00:00:00Z", &valid);
        for (name, modify) in [
            ("missing-tool", 0),
            ("missing-sample", 1),
            ("old-version", 2),
            ("metadata-only", 3),
            ("failed-download", 4),
        ] {
            let mut item = valid.clone();
            match modify {
                0 => item.tool_version.clear(),
                1 => item.sample.clear(),
                2 => item.app_version = "0.0.1".into(),
                3 => item.scope = "metadata".into(),
                _ => item.result = "failed".into(),
            }
            insert_evidence(&store, name, "1970-01-01", &item);
        }
        insert_evidence(&store, "invalid-date", "2025-02-29", &valid);
        insert_evidence(&store, "future-date", "9999-01-01", &valid);
        insert_evidence(&store, "unknown-date", "", &valid);
    }
    let store = Store::open(directory.path().to_owned()).unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let proven = store
            .platform_compatibility_get("custom-verified")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(proven.level, SupportLevel::Verified);
        assert_eq!(proven.verification.as_ref(), Some(&valid));
        for name in [
            "missing-tool",
            "missing-sample",
            "old-version",
            "metadata-only",
            "failed-download",
            "invalid-date",
            "future-date",
            "unknown-date",
        ] {
            assert_eq!(
                store
                    .platform_compatibility_get(name)
                    .await
                    .unwrap()
                    .unwrap()
                    .level,
                SupportLevel::Experimental,
                "{name}"
            );
        }
        let defaults = store.platform_compatibility_list().await.unwrap();
        assert!(defaults
            .iter()
            .filter(|item| item.platform != "custom-verified")
            .all(|item| item.level != SupportLevel::Verified));
    });
}

#[test]
fn corrupt_evidence_is_unverified_and_migration_failure_rolls_back() {
    let directory = tempfile::tempdir().unwrap();
    {
        let store = Store::open(directory.path().to_owned()).unwrap();
        store.connection.blocking_lock().execute("UPDATE platform_compatibility SET level='verified',verification_json='broken-json' WHERE platform='youtube'", []).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            assert_eq!(
                store
                    .platform_compatibility_get("youtube")
                    .await
                    .unwrap()
                    .unwrap()
                    .level,
                SupportLevel::Experimental
            );
        });
    }
    let blocked = tempfile::tempdir().unwrap();
    {
        let connection = old_database(blocked.path());
        connection.execute_batch("INSERT INTO platform_compatibility(platform,level,notes) VALUES('douyin','verified','抖音短链、图集、单视频全功能正常支持'); CREATE TRIGGER reject_update BEFORE UPDATE ON platform_compatibility BEGIN SELECT RAISE(ABORT,'blocked'); END;").unwrap();
    }
    assert!(Store::open(blocked.path().to_owned()).is_err());
    let connection = Connection::open(blocked.path().join("lumaget.db")).unwrap();
    let count: i64 = connection.query_row("SELECT COUNT(*) FROM pragma_table_info('platform_compatibility') WHERE name='verification_json'", [], |r| r.get(0)).unwrap();
    assert_eq!(count, 0);
    let original: String = connection
        .query_row(
            "SELECT level FROM platform_compatibility WHERE platform='douyin'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(original, "verified");
}
