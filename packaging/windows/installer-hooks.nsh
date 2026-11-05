; Every bundled file is required. NSIS otherwise defaults to Ignore for failed
; File instructions in silent mode, allowing an incomplete install to exit 0.
; Keep Cancel/Retry in the UI; silent installs abort with a nonzero exit code.
AllowSkipFiles off
