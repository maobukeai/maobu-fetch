//! Actual production HLS modules; no task adapter/database in this standalone harness.
#[path = "../../src-tauri/src/media_cookies/scope.rs"] mod media_cookies;
#[path = "../../src-tauri/src/m3u8/parser.rs"] mod parser;
#[path = "../../src-tauri/src/m3u8/media_parser.rs"] mod media_parser;
#[path = "../../src-tauri/src/m3u8/crypto.rs"] mod crypto;
#[path = "../../src-tauri/src/m3u8/request.rs"] mod request;
#[path = "../../src-tauri/src/m3u8/resume.rs"] mod resume;
#[path = "../../src-tauri/src/m3u8/merge.rs"] mod merge;
#[path = "../../src-tauri/src/m3u8/engine.rs"] mod engine;
#[path = "../../src-tauri/src/m3u8/tests.rs"] mod parser_tests;
#[path = "../../src-tauri/src/m3u8/behavior_tests/mod.rs"] mod behavior_tests;
