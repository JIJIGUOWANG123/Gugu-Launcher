; 咕咕启动器 NSIS 安装/卸载钩子（tauri.conf.json -> bundle.windows.nsis.installerHooks）
;
; 用户要求：卸载 = 整个安装目录连同 .mc 用户数据一起删干净，不做「是否保留数据」的询问。
; Tauri 2.12 的 NSIS 配置 schema 里没有 deleteAppDataOnUninstall 这一项（只有 template /
; installMode / languages / installerHooks 等），所以「全删」只能靠这两个钩子实现。
;
; 为什么分两段：
;   PREUNINSTALL  先杀掉可能还在跑的 GuguLauncher.exe（否则 .mc 里的 db/log 被占用删不掉），
;                 再把 .mc 整棵树删掉；
;   POSTUNINSTALL 交给 NSIS 删完自己的文件后，再 RMDir /r "$INSTDIR" 兜底清空整个目录。
;                 此时卸载器已把自身复制到 %TEMP% 运行，删除 $INSTDIR 不会卡住卸载流程。

!macro NSIS_HOOK_PREUNINSTALL
  ; 容错：进程不在时 taskkill 返回非 0，忽略即可
  nsExec::Exec 'taskkill /F /IM GuguLauncher.exe'
  Pop $0
  RMDir /r "$INSTDIR\.mc"
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  RMDir /r "$INSTDIR"
!macroend
