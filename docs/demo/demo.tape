# Regenerate with: vhs docs/demo/demo.tape   (from the repo root, after cargo build --release)
# Everything runs in a throwaway fake home under /tmp/tidy-demo.
Output "docs/demo.gif"
Set Width 1560
Set Height 960
Source "docs/demo/common.tape"

# 1. A messy Downloads folder (note the duplicate invoice)
Type "ls"
Sleep 300ms
Enter
Sleep 2.5s

# 2. Preview: a dry run is the default
Type "clear; tidy | head -29"
Sleep 300ms
Enter
Sleep 6s

# 3. Why is that copy going to the Trash?
Type `clear; tidy explain "invoice-2026-03 (1).pdf"`
Sleep 300ms
Enter
Sleep 4.5s

# 4. Apply it
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

# 5. Changed your mind? One command puts everything back.
Type "clear; tidy undo | head -7 && ls"
Sleep 300ms
Enter
Sleep 5s
