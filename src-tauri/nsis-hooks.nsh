; v0.2.2 升级桥接：旧 Win32 版把安装目录写在 luodaoyi 键下，
; 而独立仓库的新发布者键是 creamtea47。保留旧键可让本次安装
; 正确调用旧卸载器；安装完成后同步新键，后续版本切换回新发布者
; 时也能继续获取正确的卸载目录。此文件不读写任何认证信息。
!macro NSIS_HOOK_POSTINSTALL
  WriteRegStr SHCTX "Software\creamtea47\CodexUsageBar" "" "$INSTDIR"
!macroend
