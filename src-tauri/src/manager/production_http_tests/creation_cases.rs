use super::super::*;
use super::{fixture::{Fixture,Mode},harness::Harness};

pub async fn run(app:&AppHandle){
    eprintln!("native manager: real add, atomic naming intent, collision reservation and download");
    let fixture=Fixture::new(Mode::Good).await;
    let harness=Harness::new(app,None).await;
    let request:NewTaskRequest=serde_json::from_value(serde_json::json!({
        "url":fixture.url.clone(),"file_name":"created.bin","destination":harness.output.to_string_lossy(),
        "headers":{},"scheduled_at":null,"priority":0,"expected_checksum":hex::encode(Sha256::digest(fixture.expected())),
        "source":"desktop","per_task_speed_limit":0,"collision_policy":"rename","completion_action":"none",
        "media":null,"connection_count":1,"start_paused":true,"user_edited_file_name":true,
        "cloud_refresh":null,"method":null,"body":null
    })).unwrap();
    let first=harness.manager.add(request.clone()).await.unwrap();
    let second=harness.manager.add(request).await.unwrap();
    assert_eq!(first.file_name,"created.bin");
    assert_ne!(first.file_name,second.file_name);
    assert_eq!(harness.manager.store.get_task_name_intent(&first.id).await.unwrap(),Some(true));
    assert_eq!(harness.manager.list().await.unwrap().len(),2);
    assert_eq!(first.status,TaskStatus::Paused);
    harness.manager.action(&first.id,"resume").await.unwrap();
    harness.manager.dispatch_once().await.unwrap();
    harness.check_output(&first.id,&fixture.expected()).await;
    harness.manager.shutdown_bt().await.unwrap();
}
