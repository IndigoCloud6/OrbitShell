# -*- coding: utf-8 -*-
# makensis /DSTAGE_DIR=<absolute staging directory> /DSETUP_EXE=<absolute installer path> OrbitShell.nsi
Unicode true
; Adapted from the Tauri v2 NSIS installer template.
; https://github.com/tauri-apps/tauri/blob/dev/crates/tauri-bundler/src/bundle/windows/nsis/installer.nsi
; OrbitShell is staged and compiled directly with makensis, so Handlebars values
; and Tauri-only plugins are replaced with concrete OrbitShell values.
; Use per-monitor DPI rendering like Tauri's Windows NSIS template.
ManifestDPIAware true
ManifestDPIAwareness PerMonitorV2
RequestExecutionLevel admin

!include "MUI2.nsh"
!include "LogicLib.nsh"
!include "x64.nsh"

!define PRODUCT "OrbitShell"
!define VERSION "0.2.1"
!define UNINSTALL_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\OrbitShellGPUI"
!define MUI_STARTMENUPAGE_DEFAULTFOLDER "${PRODUCT}"

!define MUI_BGCOLOR "F4F7FB"
!define MUI_TEXTCOLOR "182A3B"
!define MUI_ABORTWARNING
!define MUI_HEADERIMAGE
!define MUI_HEADERIMAGE_RIGHT
!define MUI_HEADERIMAGE_BITMAP "${__FILEDIR__}\assets\OrbitShellHeader.bmp"
!define MUI_HEADERIMAGE_UNBITMAP "${__FILEDIR__}\assets\OrbitShellHeader.bmp"
!define MUI_WELCOMEFINISHPAGE_BITMAP "${__FILEDIR__}\assets\OrbitShellWizard.bmp"
!define MUI_UNWELCOMEFINISHPAGE_BITMAP "${__FILEDIR__}\assets\OrbitShellWizard.bmp"
!define MUI_WELCOMEPAGE_TITLE "欢迎使用 OrbitShell"
!define MUI_WELCOMEPAGE_TEXT "安全连接远程主机，集中管理终端、文件和 AI 工具。$\r$\n$\r$\n单击“下一步”开始安装。"
!define MUI_FINISHPAGE_TITLE "OrbitShell 已准备就绪"
!define MUI_FINISHPAGE_TEXT "安装已完成。你可以从桌面或开始菜单启动 OrbitShell。"

Var StartMenuFolder

Name "${PRODUCT} ${VERSION}"
OutFile "${SETUP_EXE}"
InstallDir "$PROGRAMFILES64\${PRODUCT}"
InstallDirRegKey HKLM "${UNINSTALL_KEY}" "InstallLocation"
SetCompressor /SOLID lzma
CRCCheck force

!define MUI_ICON "${STAGE_DIR}\orbitshell.ico"
!define MUI_UNICON "${STAGE_DIR}\orbitshell.ico"
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_LICENSE "${STAGE_DIR}\payload\LICENSE"
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_STARTMENU Application $StartMenuFolder
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "SimpChinese"
SetFont /LANG=${LANG_SIMPCHINESE} "Microsoft YaHei UI" 9

VIProductVersion "0.2.1.0"
VIAddVersionKey /LANG=${LANG_SIMPCHINESE} "ProductName" "${PRODUCT}"
VIAddVersionKey /LANG=${LANG_SIMPCHINESE} "FileDescription" "OrbitShell Windows x64 安装程序"
VIAddVersionKey /LANG=${LANG_SIMPCHINESE} "FileVersion" "${VERSION}"
VIAddVersionKey /LANG=${LANG_SIMPCHINESE} "ProductVersion" "${VERSION}"
VIAddVersionKey /LANG=${LANG_SIMPCHINESE} "OriginalFilename" "OrbitShell-windows-x64-setup.exe"
VIAddVersionKey /LANG=${LANG_SIMPCHINESE} "LegalCopyright" "Copyright (c) OrbitShell contributors"

Function .onInit
    SetShellVarContext all
    ${IfNot} ${RunningX64}
        MessageBox MB_OK|MB_ICONSTOP "此安装程序需要 64 位 Windows。" /SD IDOK
        Abort
    ${EndIf}
    IfFileExists "$INSTDIR\OrbitShell.exe" 0 install_init_done
    ExecWait '"$INSTDIR\OrbitShell.exe" --exit-for-update' $0
    ${If} $0 == 2
        Abort
    ${ElseIf} $0 != 0
        MessageBox MB_OK|MB_ICONEXCLAMATION "OrbitShell 尚未退出。请完成退出提示，或从系统托盘退出后重试安装。" /SD IDOK
        Abort
    ${EndIf}
install_init_done:
FunctionEnd

Section "OrbitShell" SEC_APP
    SetShellVarContext all
    SetOutPath "$INSTDIR"
    SetOverwrite on
    File "${STAGE_DIR}\payload\OrbitShell.exe"
    File "${STAGE_DIR}\payload\Xshell.exe"
    File "${STAGE_DIR}\payload\README.txt"
    File "${STAGE_DIR}\payload\LICENSE"
    File "${STAGE_DIR}\payload\OrbitShell-Licenses.zip"
    WriteUninstaller "$INSTDIR\Uninstall.exe"

    CreateShortcut "$DESKTOP\OrbitShell.lnk" "$INSTDIR\OrbitShell.exe"
    !insertmacro MUI_STARTMENU_WRITE_BEGIN Application
        CreateDirectory "$SMPROGRAMS\$StartMenuFolder"
        CreateShortcut "$SMPROGRAMS\$StartMenuFolder\OrbitShell.lnk" "$INSTDIR\OrbitShell.exe"
        CreateShortcut "$SMPROGRAMS\$StartMenuFolder\卸载 OrbitShell.lnk" "$INSTDIR\Uninstall.exe"
    !insertmacro MUI_STARTMENU_WRITE_END

    WriteRegStr HKLM "${UNINSTALL_KEY}" "DisplayName" "${PRODUCT}"
    WriteRegStr HKLM "${UNINSTALL_KEY}" "DisplayVersion" "${VERSION}"
    WriteRegStr HKLM "${UNINSTALL_KEY}" "Publisher" "OrbitShell contributors"
    WriteRegStr HKLM "${UNINSTALL_KEY}" "URLInfoAbout" "https://github.com/IndigoCloud6/OrbitShell"
    WriteRegStr HKLM "${UNINSTALL_KEY}" "DisplayIcon" "$INSTDIR\OrbitShell.exe,0"
    WriteRegStr HKLM "${UNINSTALL_KEY}" "InstallLocation" "$INSTDIR"
    WriteRegStr HKLM "${UNINSTALL_KEY}" "UninstallString" '"$INSTDIR\Uninstall.exe"'
    WriteRegStr HKLM "${UNINSTALL_KEY}" "QuietUninstallString" '"$INSTDIR\Uninstall.exe" /S'
    WriteRegDWORD HKLM "${UNINSTALL_KEY}" "NoModify" 1
    WriteRegDWORD HKLM "${UNINSTALL_KEY}" "NoRepair" 1
    SectionGetSize ${SEC_APP} $0
    WriteRegDWORD HKLM "${UNINSTALL_KEY}" "EstimatedSize" $0
SectionEnd

Function un.onInit
    SetShellVarContext all
    !insertmacro MUI_STARTMENU_GETFOLDER Application $StartMenuFolder
FunctionEnd

Section "Uninstall"
    SetShellVarContext all
    ClearErrors
    Delete "$INSTDIR\OrbitShell.exe"
    Delete "$INSTDIR\Xshell.exe"
    ${If} ${Errors}
        MessageBox MB_OK|MB_ICONEXCLAMATION "请从系统托盘完全退出 OrbitShell，关闭 Xshell 转接器后再卸载。" /SD IDOK
        Abort
    ${EndIf}
    Delete "$INSTDIR\README.txt"
    Delete "$INSTDIR\LICENSE"
    Delete "$INSTDIR\OrbitShell-Licenses.zip"
    Delete "$DESKTOP\OrbitShell.lnk"
    Delete "$SMPROGRAMS\$StartMenuFolder\OrbitShell.lnk"
    Delete "$SMPROGRAMS\$StartMenuFolder\卸载 OrbitShell.lnk"
    RMDir "$SMPROGRAMS\$StartMenuFolder"
    DeleteRegKey HKLM "${UNINSTALL_KEY}"
    Delete "$INSTDIR\Uninstall.exe"
    RMDir "$INSTDIR"
SectionEnd
