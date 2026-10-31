//! Bounded streaming merger for two explicitly identified, clear fMP4 tracks.
use std::path::Path;
use tokio_util::sync::CancellationToken;
#[path = "media_muxer/boxes.rs"]
mod boxes;
#[path = "media_muxer/fragments.rs"]
mod fragments;
#[cfg(test)]
#[path = "media_muxer/legacy_boxes.rs"]
mod legacy_boxes;
#[cfg(test)]
#[path = "media_muxer/legacy_tests.rs"]
mod legacy_tests;
#[path = "media_muxer/metadata.rs"]
mod metadata;
#[path = "media_muxer/streaming.rs"]
mod streaming;
#[cfg(test)]
#[path = "media_muxer/tests.rs"]
mod tests;
#[path = "media_muxer/validation.rs"]
mod validation;
pub use metadata::TrackKind;

pub async fn validate_fragmented_mp4_with_cancel(
    path: &Path,
    cancel: &CancellationToken,
) -> Result<(), String> {
    validation::validate(path, cancel).await
}

pub async fn merge_fragmented_mp4(video: &Path, audio: &Path, output: &Path) -> Result<(), String> {
    merge_fragmented_mp4_with_cancel(video, audio, output, &CancellationToken::new()).await
}
pub async fn merge_fragmented_mp4_with_cancel(
    video: &Path,
    audio: &Path,
    output: &Path,
    cancel: &CancellationToken,
) -> Result<(), String> {
    streaming::merge(video, audio, output, cancel).await
}
pub async fn probe_fragmented_mp4_track_kind(path: &Path) -> Result<TrackKind, String> {
    probe_fragmented_mp4_track_kind_with_cancel(path, &CancellationToken::new()).await
}
pub async fn probe_fragmented_mp4_track_kind_with_cancel(
    path: &Path,
    cancel: &CancellationToken,
) -> Result<TrackKind, String> {
    Ok(metadata::Input::open(path, cancel).await?.metadata.kind)
}
