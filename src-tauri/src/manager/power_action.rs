use crate::models::{PowerAction, PowerActionPhase, PowerActionState, TaskStatus};
use std::{
    collections::{HashMap, HashSet},
    process::Command,
};

pub const POWER_ACTION_COUNTDOWN_MILLIS: u64 = 60_000;

#[derive(Default)]
pub struct PowerActionRuntime {
    pub state: PowerActionState,
    pub target_ids: HashSet<String>,
    pub countdown_deadline: Option<u64>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum PowerActionDecision {
    Waiting,
    Blocked(String),
    Complete,
}

pub fn is_power_action_target(status: &TaskStatus) -> bool {
    matches!(
        status,
        TaskStatus::Queued
            | TaskStatus::Downloading
            | TaskStatus::Paused
            | TaskStatus::Scheduled
            | TaskStatus::Verifying
            | TaskStatus::WaitingNetwork
    )
}

pub fn power_action_decision(
    target_ids: &HashSet<String>,
    statuses: &HashMap<String, TaskStatus>,
) -> PowerActionDecision {
    if target_ids.iter().any(|id| !statuses.contains_key(id)) {
        return PowerActionDecision::Blocked("目标任务已被删除，系统操作不会执行".into());
    }
    if statuses
        .values()
        .any(|status| matches!(status, TaskStatus::Failed))
    {
        return PowerActionDecision::Blocked("存在失败任务，系统操作不会执行".into());
    }
    if statuses
        .values()
        .any(|status| matches!(status, TaskStatus::Paused))
    {
        return PowerActionDecision::Blocked("存在暂停任务，恢复后才会继续等待".into());
    }
    if statuses
        .values()
        .any(|status| matches!(status, TaskStatus::Cancelled))
    {
        return PowerActionDecision::Blocked("存在已取消任务，系统操作不会执行".into());
    }
    if statuses
        .values()
        .all(|status| matches!(status, TaskStatus::Completed))
    {
        PowerActionDecision::Complete
    } else {
        PowerActionDecision::Waiting
    }
}

pub fn power_action_remaining_seconds(deadline: u64, current: u64) -> u64 {
    deadline.saturating_sub(current).div_ceil(1_000)
}

#[cfg(target_os = "windows")]
pub fn execute_power_action(action: PowerAction) -> Result<(), String> {
    let Some(args) = power_action_command_args(action) else {
        return Ok(());
    };
    let status = Command::new("shutdown.exe")
        .args(args)
        .status()
        .map_err(|error| format!("无法启动 shutdown.exe：{error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("shutdown.exe 返回状态 {status}"))
    }
}

#[cfg(target_os = "windows")]
pub fn power_action_command_args(action: PowerAction) -> Option<&'static [&'static str]> {
    match action {
        PowerAction::Shutdown => Some(&["/s", "/t", "0"]),
        PowerAction::Hibernate => Some(&["/h"]),
        PowerAction::None => None,
    }
}

#[cfg(not(target_os = "windows"))]
pub fn execute_power_action(action: PowerAction) -> Result<(), String> {
    let _ = action;
    Err("当前系统不支持该电源操作".into())
}
