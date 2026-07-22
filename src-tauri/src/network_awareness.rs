//! Task 32：网络环境感知——计量网络检测与自动暂停决策。
//!
//! 设计要点（AGENTS.md §1 §3 §7 §8）：
//! - 不引入遥测或用户跟踪；检测结果仅用于本地暂停决策。
//! - 检测方法：Windows 平台通过 PowerShell 调用 WinRT
//!   `Windows.Networking.Connectivity.NetworkInformation.GetInternetConnectionProfile()`，
//!   再读取 `ConnectionProfile.GetConnectionCost().NetworkCostType`。
//!   `Unrestricted` 视为非计量；`Fixed` / `Variable` 视为计量；
//!   `Unknown` 与检测失败一律视为非计量（安全回退，避免误暂停用户任务）。
//! - 非 Windows 平台返回 `Ok(false)`。
//! - 失败（PowerShell 不可用、超时、解析失败）一律返回 `Ok(false)`，
//!   避免误报导致用户任务被错误暂停（AGENTS.md §7：影响文件完整性的错误必须失败；
//!   网络检测属"启发式策略"，失败时安全回退到不暂停，不破坏下载流程）。
//! - 每 60 秒检查一次（在 `lib.rs::setup` 中 `tokio::spawn`），
//!   不属于"高频轮询"（AGENTS.md §8）。
//! - 不向日志写入 Cookie / Authorization / 代理密码等敏感信息
//!   （本模块不接触这些数据，仅读取系统网络状态）。

use std::time::Duration;
use tokio::process::Command;

/// PowerShell 子进程超时时间。8 秒足够冷启动 PowerShell 并完成 WinRT 调用，
/// 超时则视为检测失败（安全回退到非计量）。
const POWERSHELL_TIMEOUT_SECS: u64 = 8;

/// 检测当前互联网连接是否为计量网络。
///
/// - Windows：通过 PowerShell 调用 WinRT API。
/// - 非 Windows：直接返回 `Ok(false)`。
/// - 检测失败（PowerShell 不可用、超时、解析失败、无连接）：返回 `Ok(false)`。
///
/// 返回 `Err(_)` 仅用于"调用方应中止"的严重错误；本实现尽量返回 `Ok(bool)`
/// 以保证定时检查不会因为偶发失败而中断。
pub async fn detect_metered_network() -> Result<bool, String> {
    #[cfg(not(windows))]
    {
        // 非 Windows 平台：猫步下载器仅面向 Windows 10/11（AGENTS.md §1），
        // 此分支理论上不会运行；返回 false 保证构建与测试可移植。
        Ok(false)
    }
    #[cfg(windows)]
    {
        detect_metered_network_windows().await
    }
}

#[cfg(windows)]
async fn detect_metered_network_windows() -> Result<bool, String> {
    // PowerShell 通过 WinRT 投影读取 NetworkCostType 枚举：
    //   Unknown = 0, Unrestricted = 1, Fixed = 2, Variable = 3
    //   - Unrestricted：不限量（非计量）
    //   - Fixed：固定上限（计量）
    //   - Variable：按量计费（计量）
    //   - Unknown：未知，视为非计量避免误暂停
    //
    // GetInternetConnectionProfile 在无网络连接时返回 null，脚本输出 "None"。
    // 任何异常都会被 try/catch 捕获并输出 "Error"。
    let script = r#"
$ErrorActionPreference = 'Stop'
try {
    $type = [Windows.Networking.Connectivity.NetworkInformation, Windows.Networking.Connectivity, ContentType=WindowsRuntime]
    $profile = $type::GetInternetConnectionProfile()
    if ($null -eq $profile) { Write-Output 'None'; exit 0 }
    $cost = $profile.GetConnectionCost()
    Write-Output $cost.NetworkCostType.ToString()
} catch {
    Write-Output 'Error'
}
"#;
    let command = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            script,
        ])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output();
    let result = tokio::time::timeout(Duration::from_secs(POWERSHELL_TIMEOUT_SECS), command).await;
    let output = match result {
        Ok(Ok(out)) => out,
        Ok(Err(_)) => return Ok(false),
        Err(_) => return Ok(false),
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    let trimmed = stdout.trim();
    if trimmed.is_empty() || trimmed == "Error" || trimmed == "None" {
        return Ok(false);
    }
    // 仅 Unrestricted 视为非计量；Fixed / Variable 视为计量；其余视为非计量。
    Ok(trimmed == "Fixed" || trimmed == "Variable")
}

/// Task 32.4：纯函数——根据设置、网络状态与用户标记判断是否应自动暂停。
///
/// 决策矩阵：
/// - 计量网络 + 开关开 + 用户未恢复 → 暂停
/// - 计量网络 + 开关关 → 不暂停
/// - 非计量网络 → 不暂停
/// - 用户已恢复（user_resumed_after_metered = true）→ 不自动暂停
///   （用户已在计量网络下手动恢复，不再自动暂停，直到网络变为非计量时清零标记）
pub fn should_pause_for_metered(
    metered_auto_pause: bool,
    is_metered: bool,
    user_resumed_after_metered: bool,
) -> bool {
    metered_auto_pause && is_metered && !user_resumed_after_metered
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pauses_when_metered_and_enabled_and_not_resumed() {
        // 计量网络 + 开关开 + 用户未恢复 → 暂停
        assert!(should_pause_for_metered(true, true, false));
    }

    #[test]
    fn does_not_pause_when_switch_off() {
        // 计量网络 + 开关关 → 不暂停
        assert!(!should_pause_for_metered(false, true, false));
    }

    #[test]
    fn does_not_pause_when_not_metered() {
        // 非计量网络 → 不暂停
        assert!(!should_pause_for_metered(true, false, false));
    }

    #[test]
    fn does_not_pause_when_user_resumed() {
        // 用户已恢复 → 不自动暂停（即使仍是计量网络）
        assert!(!should_pause_for_metered(true, true, true));
    }

    #[test]
    fn does_not_pause_when_switch_off_and_user_resumed() {
        // 开关关 + 用户已恢复：双重否决，应不暂停
        assert!(!should_pause_for_metered(false, true, true));
    }

    #[test]
    fn does_not_pause_when_not_metered_and_user_resumed() {
        // 非计量 + 用户已恢复：不暂停
        assert!(!should_pause_for_metered(true, false, true));
    }

    #[test]
    fn does_not_pause_when_all_conditions_false() {
        // 全部条件 false：不暂停
        assert!(!should_pause_for_metered(false, false, false));
    }

    /// Task 32.4：模拟计量网络检测——通过纯函数验证决策逻辑，
    /// 不依赖真实 Windows API（CI 可能不在 Windows 上运行）。
    #[test]
    fn simulated_metered_decision_matrix() {
        // (metered_auto_pause, is_metered, user_resumed_after_metered) -> expected
        let cases: &[(bool, bool, bool, bool)] = &[
            (true, true, false, true),    // 标准场景：应暂停
            (true, true, true, false),    // 用户已恢复：不暂停
            (true, false, false, false),  // 非计量：不暂停
            (true, false, true, false),   // 非计量 + 用户已恢复：不暂停
            (false, true, false, false),  // 开关关：不暂停
            (false, true, true, false),   // 开关关 + 用户已恢复：不暂停
            (false, false, false, false), // 全 false：不暂停
            (false, false, true, false),  // 全 false 但用户已恢复：不暂停
        ];
        for (auto_pause, is_metered, user_resumed, expected) in cases {
            let actual = should_pause_for_metered(*auto_pause, *is_metered, *user_resumed);
            assert_eq!(
                actual, *expected,
                "should_pause_for_metered({auto_pause}, {is_metered}, {user_resumed}) = {actual}, expected {expected}"
            );
        }
    }

    /// 非 Windows 平台 detect_metered_network 返回 false（在 CI 上验证安全回退）。
    /// Windows 平台在 CI 上可能无网络连接，也应当安全回退到 false，不报错。
    #[tokio::test]
    async fn detect_metered_network_returns_ok_on_any_platform() {
        // 此测试不假设结果，仅保证函数不 panic、不返回 Err。
        let result = detect_metered_network().await;
        assert!(
            result.is_ok(),
            "detect_metered_network should never return Err"
        );
    }
}
