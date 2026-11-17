; NSIS installer hooks (bundle.windows.nsis.installerHooks in tauri.conf.json).
;
; Adds CatConvert to Explorer's "Open with" menu for the media files it reads,
; without making it their default app, and to the "Send to" menu. Tauri's
; fileAssociations would make it the default app for every listed type, so
; they are only used on macOS and Linux (tauri.macos.conf.json and
; tauri.linux.conf.json). shell.wxs does the same for the MSI.
;
; Keep the extensions in sync with shell.wxs and MediaKind::from_extension in
; crates/core/src/media.rs.

!define CATCONVERT_PROGID "CatConvert.Media"

!macro CATCONVERT_EXTENSIONS MACRO
  !insertmacro ${MACRO} "jpg"
  !insertmacro ${MACRO} "jpeg"
  !insertmacro ${MACRO} "png"
  !insertmacro ${MACRO} "gif"
  !insertmacro ${MACRO} "webp"
  !insertmacro ${MACRO} "avif"
  !insertmacro ${MACRO} "heic"
  !insertmacro ${MACRO} "heif"
  !insertmacro ${MACRO} "tif"
  !insertmacro ${MACRO} "tiff"
  !insertmacro ${MACRO} "bmp"
  !insertmacro ${MACRO} "jxl"
  !insertmacro ${MACRO} "apng"
  !insertmacro ${MACRO} "mp4"
  !insertmacro ${MACRO} "m4v"
  !insertmacro ${MACRO} "mov"
  !insertmacro ${MACRO} "mkv"
  !insertmacro ${MACRO} "webm"
  !insertmacro ${MACRO} "avi"
  !insertmacro ${MACRO} "wmv"
  !insertmacro ${MACRO} "mpg"
  !insertmacro ${MACRO} "mpeg"
  !insertmacro ${MACRO} "mp3"
  !insertmacro ${MACRO} "m4a"
  !insertmacro ${MACRO} "aac"
  !insertmacro ${MACRO} "flac"
  !insertmacro ${MACRO} "wav"
  !insertmacro ${MACRO} "ogg"
  !insertmacro ${MACRO} "opus"
  !insertmacro ${MACRO} "aiff"
  !insertmacro ${MACRO} "wma"
!macroend

!macro CATCONVERT_ADD_OPEN_WITH EXT
  WriteRegStr SHCTX "Software\Classes\.${EXT}\OpenWithProgids" "${CATCONVERT_PROGID}" ""
!macroend

!macro CATCONVERT_REMOVE_OPEN_WITH EXT
  DeleteRegValue SHCTX "Software\Classes\.${EXT}\OpenWithProgids" "${CATCONVERT_PROGID}"
!macroend

!macro NSIS_HOOK_POSTINSTALL
  WriteRegStr SHCTX "Software\Classes\${CATCONVERT_PROGID}" "" "Media file"
  WriteRegStr SHCTX "Software\Classes\${CATCONVERT_PROGID}\DefaultIcon" "" "$\"$INSTDIR\${MAINBINARYNAME}.exe$\",0"
  WriteRegStr SHCTX "Software\Classes\${CATCONVERT_PROGID}\shell\open" "FriendlyAppName" "${PRODUCTNAME}"
  WriteRegStr SHCTX "Software\Classes\${CATCONVERT_PROGID}\shell\open\command" "" "$\"$INSTDIR\${MAINBINARYNAME}.exe$\" $\"%1$\""
  !insertmacro CATCONVERT_EXTENSIONS CATCONVERT_ADD_OPEN_WITH

  ; Explorer passes every selected file to one launch.
  CreateShortCut "$SENDTO\${PRODUCTNAME}.lnk" "$INSTDIR\${MAINBINARYNAME}.exe"

  ; SHCNE_ASSOCCHANGED: let Explorer pick up the new menu entries.
  System::Call 'shell32::SHChangeNotify(i 0x08000000, i 0, p 0, p 0)'
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  !insertmacro CATCONVERT_EXTENSIONS CATCONVERT_REMOVE_OPEN_WITH
  DeleteRegKey SHCTX "Software\Classes\${CATCONVERT_PROGID}"
  Delete "$SENDTO\${PRODUCTNAME}.lnk"
  System::Call 'shell32::SHChangeNotify(i 0x08000000, i 0, p 0, p 0)'
!macroend
