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
- Traditional Chinese version description, promotional text, keywords, and support URL saved
- Review contact fields are now populated in App Store Connect; values remain in Apple’s system

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

The external-file test exposed two failures, corrected in commit `435bd435`:

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

The app privacy policy is in `PRIVACY.md`. Its public `codex/mac-app-store`
branch URL is saved in App Store Connect for Simplified Chinese, Traditional
Chinese, and English (US). The user confirmed Apple’s privacy declaration
commitment, and the “Data Not Collected” answers were published successfully.
Move the URL to the default branch when the release changes merge. The repository README previously
called the app unusable; it now states the actual pre-release status. All
store answers must match the final signed build.

The existing Developer ID DMG pipeline and artifact are separate and were not
modified by this draft. The Mac App Store needs its own sandboxed package; the
notarized DMG is not an App Store build.

## Sandbox export acceptance (September 30)

The first external PDF export failed before publication and left the original
99-byte Markdown file unchanged. Its portable writer attempted to create a
sibling temporary file outside the selected-file sandbox grant.

The sandbox host now configures a per-task publisher: Rust stages complete bytes
in the application container, retains destination/protected-file validation and
the cancellation commit guard, then invokes Foundation to publish. The callback
receives the captured overwrite decision. New-file publication refuses an output
that appeared after the final validation. Existing output uses coordinated
Foundation replacement. The unsandboxed export path is unchanged.

Signed candidate `artifacts/appstore-export-v1/Yu.app` passed:

- Real NSSavePanel export to `Public/YuAppStoreSaveAsCheck.pdf` and confirmed
  replacement of that test PDF. PDFKit independently read one A4 page containing
  the expected document text; the output is 13,788 bytes.
- Real HTML export to the same directory; the 2,438-byte document contains the
  expected title and valid HTML declaration.
- Real single-PNG export to the same directory; ImageIO decoded 800 by 186 pixels,
  and the rendered image was visually checked.
- Signed sandbox PDF self-check in the container: snapshot text, page dimensions,
  source/selection/history preservation, and continued editing/undo passed.
- 33 export unit tests and 101 existing FFI unit tests; 3 new host-publication
  tests; export safety/write-failure tests; one additional full-job publisher
  test. Clippy with warnings denied passed for both affected libraries.

After building this candidate, the source also permits publisher registration
when print preparation has already completed, before any print publication.
The updated Rust source and full-job test compiled successfully. Rebuild the
final signed release from its committed source before upload.

Segmented PNG directories, external relative-image access/import, and real
system printing remain separate sandbox acceptance items. These successes do
not close those items. No new system permissions or settings were enabled.


## Sandbox resource and release-gate follow-up (September 30)

The external resource fixture contains a table, inline/block math, Mermaid,
relative PNG, footnote, Chinese, English and bidirectional text. Candidate
`artifacts/appstore-resources-v1/Yu.app` passed these real-panel checks:

- Opening the Markdown requests its image folder through NSOpenPanel. Only the
  disposable `Public/YuAppStoreFinalAcceptance` folder was selected.
- HTML output embeds the local PNG and no longer contains the unreadable-image
  diagnostic. The output is 26,095 bytes. PDF output is 91,048 bytes.
- A 700-paragraph document exported as five numbered PNGs at 2x. Their dimensions
  are 1600 x 10448, 1600 x 10462 (three files), and 1600 x 8888.
- Quit/relaunch and Recent Open restore the folder bookmark alongside the file
  bookmark. The local feather image renders without another permission prompt.
- Save As into a new directory copies the relative image; both Markdown and image
  bytes match the source. Inserting another image copies it into `assets`; undo,
  redo, and a second save work. The original 579-byte fixture remains unchanged.

The resource catalog comes from canonical Markdown references. Code examples and
remote URLs do not trigger local folder requests. PNG directory destination
capture now waits until split confirmation, permitting a narrowly scoped folder
grant before publication. Cancellation and existing-output protection remain
required gates.

Real system printing exposed a missing `com.apple.security.print` entitlement.
It has been added only to the main app; the helper keeps sandbox inheritance.
This is the documented Apple sandbox permission for document printing:
https://developer.apple.com/documentation/bundleresources/entitlements/com.apple.security.print
Signed candidate `artifacts/appstore-resources-v3/Yu.app` passed all eight
native system-print checks (whole/range/landscape, source/image aliases, write
denial, target change, cancellation), with zero physical jobs submitted. The
actual print panel retest is pending because the Mac locked during UI acceptance.

Rust workspace tests passed: 1,615 passed and 7 ignored. All-target clippy passes
with warnings denied after replacing unchecked test unwraps with assertions.
The dependency audit now records the existing shared syntax/resource dependencies
of yu-export, and the geometry audit records the existing C ABI document-space
source-range bounds. The macOS export module states its platform cfg internally
as well as at its declaration so the static dependency audit recognizes it.
All dependency/FFI/geometry checks and 19 clean-build native self-checks passed.
These fixes do not disable or bypass a check.

Signed candidate v3 also passed all ten native PNG cases: light 1x/2x, dark,
split cancel/confirm/table, oversized indivisible table, existing directory,
source alias, and cancellation. Existing outputs and source/history were
preserved in all refusal cases. Native checks restore only previously selected
Powerbox grants; the sandbox stays enabled. Reports are retained in
`artifacts/appstore-proof/sandbox-png-v3-report.json` and
`artifacts/appstore-proof/sandbox-print-v3-report.json`.

Generate the formal package from a clean commit after this entry. Store
screenshots, build upload, review submission, and the locked-screen UI print
retest are separate remaining items; do not label the app submitted or published.
