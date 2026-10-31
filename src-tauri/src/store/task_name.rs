//! 用户手编文件名的意图与新任务同事务保存，旧任务未知时保守保留名称。
use super::*;

pub(super) fn key(id:&str)->String {format!("task-name-intent:{id}")}
impl Store {
    pub async fn create_task_with_name_intent(&self,task:&DownloadTask,edited:bool)->Result<(),String>{
        let mut connection=self.connection.lock().await;
        let transaction=connection.transaction().map_err(|e|format!("创建任务事务失败：{e}"))?;
        Self::upsert_with(&transaction,task)?;
        transaction.execute("INSERT INTO app_state(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![key(&task.id),if edited {"true"} else {"false"}]).map_err(|e|format!("保存文件名选择意图失败：{e}"))?;
        transaction.commit().map_err(|e|format!("提交新任务失败：{e}"))?;
        Ok(())
    }
    pub async fn get_task_name_intent(&self,id:&str)->Result<Option<bool>,String>{
        let value:Option<String>=self.connection.lock().await.query_row("SELECT value FROM app_state WHERE key=?1",[key(id)],|row|row.get(0))
            .optional().map_err(|e|format!("读取文件名选择意图失败：{e}"))?;
        value.map(|value|serde_json::from_str(&value).map_err(|_|"文件名选择记录损坏，未覆盖当前文件名".into())).transpose()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn custom_name_intent_survives_restart_and_is_removed_with_the_record(){
        let root=tempfile::tempdir().unwrap();
        let data=root.path().to_owned();
        let store=tokio::task::spawn_blocking({let data=data.clone();move||Store::open(data)}).await.unwrap().unwrap();
        let task=crate::bt::tests::bt_task_for_test();
        assert_eq!(store.get_task_name_intent(&task.id).await.unwrap(),None);
        store.create_task_with_name_intent(&task,true).await.unwrap();
        drop(store);
        let store=tokio::task::spawn_blocking(move||Store::open(data)).await.unwrap().unwrap();
        assert_eq!(store.get_task_name_intent(&task.id).await.unwrap(),Some(true));
        store.remove_task(&task.id).await.unwrap();
        assert_eq!(store.get_task_name_intent(&task.id).await.unwrap(),None);
    }
    #[tokio::test]
    async fn name_intent_write_failure_rolls_back_new_task(){
        let root=tempfile::tempdir().unwrap();let data=root.path().to_owned();
        let store=tokio::task::spawn_blocking(move||Store::open(data)).await.unwrap().unwrap();
        store.connection.lock().await.execute_batch("CREATE TRIGGER reject_test_intent BEFORE INSERT ON app_state WHEN NEW.key LIKE 'task-name-intent:%' BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
        let task=crate::bt::tests::bt_task_for_test();
        assert!(store.create_task_with_name_intent(&task,false).await.is_err());
        assert!(store.get_task(&task.id).await.unwrap().is_none());
    }
}
