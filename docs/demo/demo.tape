# Regenerate with: vhs docs/demo/demo.tape   (from the repo root, after cargo build --release)
# Everything runs in a throwaway fake home under /tmp/tidy-demo.
Output "docs/demo.gif"
Set Width 1560
Set Height 940
Source "docs/demo/common.tape"

# 1. A messy Downloads folder
Type "ls"
Sleep 300ms
Enter
Sleep 2.5s

# 2. Preview: a dry run is the default
Type "clear; tidy | head -28"
Sleep 300ms
Enter
Sleep 6s

# 3. Apply it
Type "clear; tidy --apply | tail -3 && ls"
Sleep 300ms
Enter
Sleep 3s
Type "ls ~/Documents/PDFs"
Sleep 300ms
Enter
Sleep 1.5s
Type "ls ~/.Trash"
Sleep 300ms
Enter
Sleep 3s

# 4. Changed your mind? One command puts everything back.
Type "clear; tidy undo | head -7 && ls"
Sleep 300ms
Enter
Sleep 5s
