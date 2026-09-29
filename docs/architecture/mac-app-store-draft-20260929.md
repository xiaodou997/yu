# Mac App Store release checklist

The App Store Connect record is saved as a draft:

- App Store Connect ID: `6817272771`
- Team ID: `V6M88BQG7C`
- Bundle ID: `io.github.xiaodou997.yu`
- macOS version: `0.1.2` (`Prepare for Submission`)
- Price: free (`$0.00` in the United States and other listed regions)
- Release after review: manual
- Review login: not required
- Simplified Chinese: `Yu Markdown` / `所见即所得，专注写作`
- English (US): `Yu — Markdown Editor` / `Native WYSIWYG Writing`
- Traditional Chinese: `Yu: Markdown 寫作` / `所見即所得，專注寫作`
- Primary category: Productivity; age rating: 4+
- No build uploaded and no review submitted

Group 7 internationalization was merged into this branch. Complete the store
listing, screenshots, privacy information, regional availability, and submission.
The user will manage App Review contact details personally. Do not create
a Git tag or a GitHub Release until the Windows build is ready for a joint
release.

The App Store sandbox candidate is signed with Apple Distribution, including
separate app and helper entitlements. The Mac Installer Distribution certificate
is installed, and the packaging pipeline has produced and verified an Apple-signed
`.pkg`. No package has been uploaded yet.

## Sandbox document acceptance

The external-file test exposed two failures, now corrected in the working tree:

- Rust's sibling temporary file was outside a file-only Powerbox grant. The
  sandbox path now stages canonical bytes in the app container and delegates
  replacement to Foundation, preserving Rust's conflict checks.
- Save As tried to create a security-scoped bookmark before creating the file.
  It now retains the panel grant, writes the file, then persists the bookmark.
  Bookmark failure is reported separately from write failure.

Candidate `artifacts/appstore-saveas-v3/Yu.app` passed Apple Distribution signing,
secure timestamps, and strict bundle verification on macOS 27. Its signed sandbox
image-batch self-check passed first save, image paste, undo/redo, and reopen inside
the container. This is separate from external-folder image acceptance.

With a real NSSavePanel, the user saved `Public/YuAppStoreSaveAsCheck.md`.
Disk bytes were verified after initial save and a second edit/save. After quitting
and relaunching Yu, Recent Open reopened the file without a new panel grant; another
edit/save succeeded and its disk bytes were verified. Earlier testing also passed
external existing-document save, autosave, and reopen.

Regression checks passed: 27 document-session tests, 16 image Save As tests,
16 storage unit tests, 101 FFI unit tests, the standalone bookmark lifecycle test,
and localization validation for all five bundled languages (298 keys).

External-folder relative images, exports, and the remaining store acceptance
still need completion. Candidates are not upload artifacts. Rebuild from a clean
commit after the fixes are committed; the previous build-3 package predates them.

The app privacy policy is in `PRIVACY.md`; publish it on the default branch
before using its URL in App Store Connect. The repository README previously
called the app unusable; it now states the actual pre-release status. All
store answers must match the final signed build.

The existing Developer ID DMG pipeline and artifact are separate and were not
modified by this draft. The Mac App Store needs its own sandboxed package; the
notarized DMG is not an App Store build.
