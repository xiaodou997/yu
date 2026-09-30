# Mac App Store release checklist

Current state, September 30: the App Store Connect submission is **Waiting for
Review**. The formal package passed Apple validation and upload. Submission ID:
`6500fafc-8775-4ff9-9151-91f6149c17c5`.

- App Store Connect ID: `6817272771`
- Team ID: `V6M88BQG7C`
- Bundle ID: `io.github.xiaodou997.yu`
- macOS version: `0.1.2` / build `3` (`Waiting for Review`)
- Price: free (`$0.00` in the United States and other listed regions)
- Release after review: manual
- Review login: not required
- Simplified Chinese: `Yu Markdown` / `所见即所得，专注写作`
- English (US): `Yu — Markdown Editor` / `Native WYSIWYG Writing`
- Traditional Chinese: `Yu: Markdown 寫作` / `所見即所得，專注寫作`
- Primary category: Productivity; age rating: 4+
- Build delivery ID: `c964828f-ef8a-4394-b017-688ce0fbc901`
- Submitted to Apple App Review on September 30, 2026; approval and manual release remain pending
- Traditional Chinese version description, promotional text, keywords, and support URL saved
- Review contact fields are now populated in App Store Connect; values remain in Apple’s system

Group 7 internationalization was merged into this branch. The three store
localizations, screenshots, privacy information, regional availability, and
review submission are now configured.
The user will manage App Review contact details personally. Do not create
a Git tag or a GitHub Release until the Windows build is ready for a joint
release.

The App Store sandbox candidate is signed with Apple Distribution, including
separate app and helper entitlements. The Mac Installer Distribution certificate
is installed, and the packaging pipeline has produced and verified an Apple-signed
`.pkg`. The formal package from commit `ec726f99` has been uploaded and submitted.

The sections below retain the acceptance history of earlier candidates. Use
the final submission record at the end of this document for current state.

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
actual print panel retest subsequently passed on September 30: the user
completed the system PDF save action, and independent PDF inspection confirmed
two Letter pages with tables, math, Mermaid, both local images, bidirectional
text, and the footnote. The 650-byte edited fixture remained intact.

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

## Formal submission package and store state (September 30)

The formal `0.1.2` build `3` package was generated from clean source commit
`ec726f9905a82c7a045fe03804fc44362c49ec29`. It is retained at
`artifacts/appstore-submission-0.1.2-build3-ec726f99/Yu-macOS-AppStore.pkg`.
Its SHA256 is
`8a1a60f2684b7a4d35399148ac416457533e6dc0900123fa0840c92f0ceb17b0`.
Apple installer signing and the trusted timestamp were verified. Expanding the
package reproduced the entire signed-app inventory. Unsigned main/helper/Metal
code matches the tested v3 candidate. The manifest and acceptance evidence are
stored alongside the package; this source identity stays fixed when later
documentation or upload tooling changes.

App Store Connect now has availability saved for all 175 countries/regions.
Current regional prices are zero, and the version retains manual release.
All three localizations now have their own 2880 x 1800 JPEG screenshot, showing
actual signed-app document content with localized captions. The application
interface remained in the user's current Simplified Chinese language during
capture; no system language preferences were changed. Review notes explaining
local-file use and the scoped image-folder prompt were saved. No tag or GitHub
Release has been created.

`tools/upload-macos-appstore.py` verifies the formal manifest and package hash,
then performs Apple validation before upload. Run it interactively on the Mac;
enter an existing App-specific password at its hidden prompt. It passes the
password through stdin and redacts the account and password from saved output.
It does not store credentials or submit for review. `--check-only` verifies the
local package without sending it to Apple.

## Final Apple submission (September 30)

- Apple `validate-app` and `upload-package` both returned exit code 0. Delivery
  ID: `c964828f-ef8a-4394-b017-688ce0fbc901`; transferred bytes: 45,926,548.
- Warning 90889 says the bundle lacks a provisioning profile required for
  TestFlight. The package was successfully processed and submitted for the
  App Store; this does not claim TestFlight eligibility.
- Build `0.1.2 (3)` is associated with the store version. The encryption answer
  is “none of the listed algorithms”: source uses Apple's SHA256 for integrity
  fingerprints, and the locked Rust dependency sets contain no independent
  encryption/TLS packages. Apple no longer reports missing export compliance.
- Content rights are set to necessary rights for third-party resources, based
  on the audited licenses shipped in the bundle. The English description,
  promotional text, keywords, and support URL were filled and saved after the
  preflight found them empty. Traditional and Simplified Chinese fields were
  independently inspected.
- Screenshot uploads initially failed because browser screenshot bytes were
  JPEG with a `.png` filename. The original bytes were saved under matching
  `.jpg` names, uploaded, and their failed placeholders removed. Read-only
  Media Manager independently showed `zh-Hans-01.jpg`, `en-01.jpg`, and
  `zh-Hant-01.jpg` in their respective localizations before final submission.
- Apple preflight passed, then the final submit action succeeded. Submission
  `6500fafc-8775-4ff9-9151-91f6149c17c5` shows **Waiting for Review** for
  `0.1.2 (3)` at September 30, 10:11 (Australia/Perth).
- Price remains free; availability is 175 countries/regions; release remains
  manual. The app has not been approved or published. Apple review and the
  user's later manual release are the next external steps.

Local evidence: `artifacts/appstore-submission-0.1.2-build3-ec726f99/` contains
the package, manifest, acceptance evidence, redacted validation/upload logs,
and upload result. `artifacts/appstore-proof/` retains the rendered acceptance
and browser submission evidence; store images are in
`artifacts/appstore-screenshots/`. Credentials remain local and were not saved
by the upload script.
