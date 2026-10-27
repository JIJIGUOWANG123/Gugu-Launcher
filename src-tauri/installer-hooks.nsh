; 咕嘎启动器 NSIS 安装/卸载钩子（tauri.conf.json -> bundle.windows.nsis.installerHooks）
;
; 用户要求：
;   1) 手动双击 / 静默安装要**覆盖已有安装**（沿用上次安装目录），不要装成第二份；
;   2) 卸载 = 整个安装目录连同 .mc 用户数据一起删干净。
;
; 三段钩子：
;   PREINSTALL    读注册表 Uninstall\${PRODUCTNAME}\InstallLocation（HKCU → HKLM → WOW6432Node），
;                 只有当「当前目录还是默认目录」且「旧目录存在」时才把 $INSTDIR 指回旧目录。
;   PREUNINSTALL  先杀进程再删 .mc；
;   POSTUNINSTALL 兜底 RMDir /r $INSTDIR。
;
; 本文件在 installer.nsi 顶部 !include（早于 ${PRODUCTNAME} 等 !define），
; 所以所有 ${...} 都写在 macro 内部，插入时才展开。

!macro NSIS_HOOK_PREINSTALL
  Push $R0
  Push $R1
  Push $R2
  Push $R3

  FileOpen $R3 "$TEMP\gugu-hook.log" a
  FileWrite $R3 "--- PREINSTALL enter INSTDIR=$INSTDIR$\r$\n"
  FileClose $R3

  ; ---------- 1) 读 InstallLocation：HKCU → HKLM → WOW6432Node ----------
  StrCpy $R0 ""
  ReadRegStr $R1 HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\${PRODUCTNAME}" "InstallLocation"
  ${If} $R1 == ""
    ReadRegStr $R1 HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\${PRODUCTNAME}" "InstallLocation"
  ${EndIf}
  ${If} $R1 == ""
    ReadRegStr $R1 HKLM "Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\${PRODUCTNAME}" "InstallLocation"
  ${EndIf}
  ${If} $R1 == ""
    ReadRegStr $R1 HKCU "Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\${PRODUCTNAME}" "InstallLocation"
  ${EndIf}

  ; ---------- 2) 去首尾引号与尾随反斜杠 ----------
  ${If} $R1 != ""
    StrCpy $R0 $R1
    gugu_strip_loop:
      StrCpy $R2 $R0 1
      ${If} $R2 == '"'
        StrCpy $R0 $R0 "" 1
        Goto gugu_strip_loop
      ${EndIf}
      StrCpy $R2 $R0 1 -1
      ${If} $R2 == '"'
        StrCpy $R0 $R0 -1
        Goto gugu_strip_loop
      ${EndIf}
      ${If} $R2 == "\"
        StrCpy $R0 $R0 -1
        Goto gugu_strip_loop
      ${EndIf}
  ${EndIf}

  ; ---------- 3) 判定：旧目录可用 + 当前目录没被用户改过 ----------
  ; 没被改过 = $INSTDIR 还是模板给的默认目录（currentUser / perMachine 三种都算）
  StrCpy $R2 "$LOCALAPPDATA\${PRODUCTNAME}"

  FileOpen $R3 "$TEMP\gugu-hook.log" a
  FileWrite $R3 "  raw=$R1 stripped=$R0 default=$R2 cur=$INSTDIR$\r$\n"
  FileClose $R3

  ${If} $R0 == ""
    FileOpen $R3 "$TEMP\gugu-hook.log" a
    FileWrite $R3 "  branch=no-key（注册表没有 InstallLocation）$\r$\n"
    FileClose $R3
    DetailPrint "未找到上次安装目录（注册表无 InstallLocation），使用当前目录：$INSTDIR"
  ${Else}
    ; 当前目录是不是默认值？（三种默认都算）
    StrCpy $R1 "0"
    ${If} $INSTDIR == $R2
      StrCpy $R1 "1"
    ${EndIf}
    ${If} $INSTDIR == "$PROGRAMFILES\${PRODUCTNAME}"
      StrCpy $R1 "1"
    ${EndIf}
    ${If} $INSTDIR == "$PROGRAMFILES64\${PRODUCTNAME}"
      StrCpy $R1 "1"
    ${EndIf}

    ${If} $R1 != "1"
      FileOpen $R3 "$TEMP\gugu-hook.log" a
      FileWrite $R3 "  branch=user-custom（当前目录不是默认值，尊重用户选择）$\r$\n"
      FileClose $R3
      DetailPrint "当前目录不是默认值（$INSTDIR），不改写；注册表里的旧目录：$R0"
    ${Else}
      ; 旧目录必须真的存在：先看 $R0\GuguLauncher.exe，退一步看目录本身
      IfFileExists "$R0\GuguLauncher.exe" gugu_old_ok
      IfFileExists "$R0\*.*" gugu_old_ok

      FileOpen $R3 "$TEMP\gugu-hook.log" a
      FileWrite $R3 "  branch=old-missing（旧目录不存在）$\r$\n"
      FileClose $R3
      DetailPrint "上次安装目录已不存在（$R0），按当前目录安装：$INSTDIR"
      Goto gugu_preinstall_done

      gugu_old_ok:
      StrCpy $INSTDIR $R0
      SetOutPath $INSTDIR
      DetailPrint "沿用上次安装目录：$INSTDIR（注册表值 $R1）"
      ; 模板在钩子之前已经 SetOutPath 过一次默认目录，会留下一个空目录：
      ; RMDir 不带 /r，只在目录为空时删掉，绝不误删真实数据
      RMDir "$LOCALAPPDATA\${PRODUCTNAME}"
      RMDir "$PROGRAMFILES\${PRODUCTNAME}"
      RMDir "$PROGRAMFILES64\${PRODUCTNAME}"
      WriteRegStr SHCTX "${MANUPRODUCTKEY}" "" "$INSTDIR"
      FileOpen $R3 "$TEMP\gugu-hook.log" a
      FileWrite $R3 "  branch=relocated INSTDIR=$INSTDIR$\r$\n"
      FileClose $R3
      Goto gugu_preinstall_done
    ${EndIf}
  ${EndIf}

  gugu_preinstall_done:

  Pop $R3
  Pop $R2
  Pop $R1
  Pop $R0
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  nsExec::Exec 'taskkill /F /IM GuguLauncher.exe'
  Pop $0
  RMDir /r "$INSTDIR\.mc"
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  RMDir /r "$INSTDIR"
!macroend
