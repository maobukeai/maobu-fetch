; ---------------------------------------------------------------------------
; 猫步下载器 – NSIS 安装器 Hooks
; 遵循 AGENTS.md: 保留注册表键名 app.lumaget.desktop, 端口 17433 不变
; ---------------------------------------------------------------------------

; ── 安装后：按安装选项页勾选写入开机自启注册表 ────────────
; $AutoStartState 由安装向导选项页设置（默认勾选；静默/Passive 安装保持自启）。
!macro NSIS_HOOK_POSTINSTALL
  ${If} $AutoStartState <> 0
    WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Run" \
      "app.lumaget.desktop" '"$INSTDIR\${MAINBINARYNAME}.exe"'
  ${EndIf}
!macroend

; ── 卸载后：清理开机自启注册表 ──────────────────────────
!macro NSIS_HOOK_POSTUNINSTALL
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" \
    "app.lumaget.desktop"
!macroend
