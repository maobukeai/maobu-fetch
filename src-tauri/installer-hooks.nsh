; ---------------------------------------------------------------------------
; 猫步下载器 – NSIS 安装器 Hooks
; 遵循 AGENTS.md: 保留注册表键名 app.lumaget.desktop, 端口 17433 不变
; ---------------------------------------------------------------------------

; ── 安装后：写入开机自启注册表 ──────────────────────────
!macro NSIS_HOOK_POSTINSTALL
  ; 默认启用开机自启 —— 用户可在程序"设置 → 应用行为"中随时关闭
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Run" \
    "app.lumaget.desktop" '"$INSTDIR\${MAINBINARYNAME}.exe"'
!macroend

; ── 卸载后：清理开机自启注册表 ──────────────────────────
!macro NSIS_HOOK_POSTUNINSTALL
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" \
    "app.lumaget.desktop"
!macroend
