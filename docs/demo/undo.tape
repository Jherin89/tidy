Output "/tmp/tidy-undo.gif"
Set Width 1560
Set Height 1400
Source "docs/demo/common.tape"
Hide
Type "tidy --apply > /dev/null && clear"
Enter
Sleep 1s
Show
Type "tidy undo --dry-run"
Enter
Sleep 2s
Type "tidy undo"
Enter
Sleep 2s
Screenshot "docs/undo.png"
Sleep 200ms
