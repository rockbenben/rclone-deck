; Use a fresh Explorer identity without changing the bundle/install identity.
!macro UseDeckShellId
  !define DECK_INSTALL_BUNDLE_ID "${BUNDLEID}"
  !undef BUNDLEID
  !define BUNDLEID "${DECK_INSTALL_BUNDLE_ID}.Desktop"
!macroend

!macro RestoreDeckBundleId
  !undef BUNDLEID
  !define BUNDLEID "${DECK_INSTALL_BUNDLE_ID}"
  !undef DECK_INSTALL_BUNDLE_ID
!macroend

!macro SetDeckShortcutIcon path
  ${If} ${FileExists} "${path}"
    !insertmacro IsShortcutTarget "${path}" "$INSTDIR\${MAINBINARYNAME}.exe"
    Pop $0
    ${If} $0 = 1
      CreateShortcut "${path}" "$INSTDIR\${MAINBINARYNAME}.exe" "" "$INSTDIR\rclone-deck-taskbar.ico" 0
      !insertmacro UseDeckShellId
      !insertmacro SetLnkAppUserModelId "${path}"
      !insertmacro RestoreDeckBundleId
    ${EndIf}
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  ${If} $UpdateMode != 1
    !insertmacro UseDeckShellId
    !insertmacro DeleteAppUserModelId
    !insertmacro RestoreDeckBundleId
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTINSTALL
  Push $0
  !if "${STARTMENUFOLDER}" != ""
    !insertmacro SetDeckShortcutIcon "$SMPROGRAMS\$AppStartMenuFolder\${PRODUCTNAME}.lnk"
  !else
    !insertmacro SetDeckShortcutIcon "$SMPROGRAMS\${PRODUCTNAME}.lnk"
  !endif
  !insertmacro SetDeckShortcutIcon "$DESKTOP\${PRODUCTNAME}.lnk"
  Pop $0
!macroend
