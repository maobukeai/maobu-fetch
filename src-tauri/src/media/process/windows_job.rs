use std::{ffi::c_void, mem::size_of, ptr};
use tokio::process::{Child, Command};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE},
    System::{
        Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32,
        },
        JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
            SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        },
        Threading::{
            OpenThread, ResumeThread, CREATE_NO_WINDOW, CREATE_SUSPENDED, THREAD_SUSPEND_RESUME,
        },
    },
};

pub(super) struct Job(HANDLE);
// Windows kernel handles are valid across threads; this handle is owned and only closed on drop.
unsafe impl Send for Job {}

impl Job {
    pub fn prepare(command: &mut Command) {
        command.creation_flags(CREATE_NO_WINDOW | CREATE_SUSPENDED);
    }

    pub fn attach_and_resume(child: &Child) -> Result<Self, String> {
        let process = child.raw_handle().ok_or("媒体进程已退出，无法绑定任务")?;
        let process_id = child.id().ok_or("媒体进程没有有效编号")?;
        unsafe {
            let handle = CreateJobObjectW(ptr::null(), ptr::null());
            if handle.is_null() {
                return Err("无法创建媒体进程隔离任务".into());
            }
            let job = Self(handle);
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                &limits as *const _ as *const c_void,
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            ) == 0
                || AssignProcessToJobObject(handle, process as HANDLE) == 0
            {
                return Err("无法隔离媒体进程与合并子进程，已停止启动".into());
            }
            resume_primary_thread(process_id)?;
            Ok(job)
        }
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

unsafe fn resume_primary_thread(process_id: u32) -> Result<(), String> {
    let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
    if snapshot == INVALID_HANDLE_VALUE {
        return Err("无法获取媒体进程启动线程".into());
    }
    let mut thread: THREADENTRY32 = std::mem::zeroed();
    thread.dwSize = size_of::<THREADENTRY32>() as u32;
    let mut found = Thread32First(snapshot, &mut thread) != 0;
    let mut result = Err("媒体进程启动线程不存在".to_owned());
    while found {
        if thread.th32OwnerProcessID == process_id {
            let handle = OpenThread(THREAD_SUSPEND_RESUME, 0, thread.th32ThreadID);
            if !handle.is_null() {
                let resumed = ResumeThread(handle);
                CloseHandle(handle);
                result = if resumed == u32::MAX {
                    Err("无法恢复媒体进程启动".into())
                } else {
                    Ok(())
                };
            }
            break;
        }
        found = Thread32Next(snapshot, &mut thread) != 0;
    }
    CloseHandle(snapshot);
    result
}
