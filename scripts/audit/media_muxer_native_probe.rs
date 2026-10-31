//! Compile the production fMP4 muxer and atomic publication module directly.
use std::path::Path;
use tokio::fs;
use uuid::Uuid;
#[path="../../src-tauri/src/manager/file_publication.rs"]
mod file_publication;
mod manager {pub(crate) use super::file_publication::publish_file_atomically;}
#[path="../../src-tauri/src/media_muxer.rs"]
mod media_muxer;
#[cfg(not(test))]
fn main(){
    let paths:Vec<_>=std::env::args_os().skip(1).map(std::path::PathBuf::from).collect();
    if paths.len()==2 && paths[0]==std::path::Path::new("--validate") {
        let runtime=tokio::runtime::Runtime::new().expect("test runtime");
        if let Err(error)=runtime.block_on(media_muxer::validate_fragmented_mp4_with_cancel(&paths[1],&tokio_util::sync::CancellationToken::new())){eprintln!("{error}");std::process::exit(1);}
        println!("clear fMP4 validated");return;
    }
    if paths.len()!=3{eprintln!("provide video audio output paths");std::process::exit(2);}
    let runtime=tokio::runtime::Runtime::new().expect("test runtime");
    match runtime.block_on(media_muxer::merge_fragmented_mp4(&paths[0],&paths[1],&paths[2])){
        Ok(())=>{runtime.block_on(media_muxer::validate_fragmented_mp4_with_cancel(&paths[2],&tokio_util::sync::CancellationToken::new())).expect("validate generated fMP4");println!("merged and validated")},Err(error)=>{eprintln!("{error}");std::process::exit(1);}
    }
}
