# Mac App Store release checklist

## Current state (September 30, 2026)

Yu `0.1.2 (3)` has passed Apple validation and upload and is **Waiting for
Review**. Approval and public availability remain pending. Pricing is free;
release after approval is manual. Do not create a Git tag or GitHub Release
until Windows is ready for the joint release.

- Bundle ID: `io.github.xiaodou997.yu`
- Minimum macOS: 26.0; acceptance host: macOS 27, Apple Silicon
- Bundled languages: Simplified Chinese, Traditional Chinese, English,
  Japanese, and Korean
- Store pages: Simplified Chinese, Traditional Chinese, and English (US)
- Simplified Chinese: `Yu Markdown` / `所见即所得，专注写作`
- English: `Yu — Markdown Editor` / `Native WYSIWYG Writing`
- Traditional Chinese: `Yu: Markdown 寫作` / `所見即所得，專注寫作`
- Category: Productivity; age rating: 4+
- Public privacy policy: [PRIVACY.md](../../PRIVACY.md); published store
  declaration: Data Not Collected
- Review contact details are maintained in App Store Connect.

Developer identities, credentials, delivery/submission identifiers, and browser
account evidence belong in local records under the ignored `artifacts/`
directory. They are omitted from this public checklist.

## Sandbox behavior

The App Store build uses Apple Distribution signing and separate app/helper
entitlements. The helper inherits the sandbox. Only the main app receives the
document-printing entitlement. Developer ID direct distribution retains its
separate signing and notarized DMG pipeline.

### Documents

File-panel grants cover selected files and do not permit arbitrary sibling
temporary files. Rust stages canonical document bytes in the application
container; Foundation publishes them while preserving conflict checks. New
Save As destinations retain the panel grant until the write completes and only
then create a persistent bookmark. Bookmark failures are reported separately
from write failures.

Real file-panel acceptance passed opening an external document, first save,
Save As, subsequent editing/saving, autosave, quit/relaunch, and reopening from
Recent Open without another panel grant. Disk contents were independently
verified after saves.

### Export and images

The sandbox export publisher stages complete bytes in the container and uses
Foundation for destination publication. Rust retains destination validation,
the captured overwrite decision, source protection, and cancellation checks.
Creating a new output refuses a file that appeared after validation; existing
outputs use coordinated replacement.

Real external PDF, HTML, single PNG, and five-part PNG exports passed. PDFKit
and ImageIO independently decoded outputs. Export refusal and cancellation
checks preserved source text, editing history, and existing outputs.

Local image references request a scoped folder grant when necessary. Remote
URLs and code examples do not trigger local image-folder requests. Acceptance
passed relative-image loading, bookmark restoration after relaunch, Save As
with image copying, image insertion, undo/redo, and subsequent saving.

### System printing

All eight signed sandbox print checks passed: whole document, range, landscape,
source/image aliases, denied writes, changed targets, and cancellation. Real
system printing to PDF also passed. Independent inspection confirmed two pages
containing tables, formulas, Mermaid, local images, bidirectional text, and a
footnote; the source document remained intact.

## Validation and formal package

- Rust workspace: 1,615 passed, 7 ignored; formatting and clippy passed.
- Dependency, FFI, and geometry gates passed.
- Clean build: 19 native self-checks passed; all five bundled languages passed
  localization validation.
- Signed sandbox: all ten PNG and eight print checks passed.
- App/helper signatures, sandbox entitlements, Hardened Runtime, secure
  timestamps, and installer signing were verified.
- Expanded installer contents matched the full signed-app inventory.

The submitted package remains tied to clean source commit
`ec726f9905a82c7a045fe03804fc44362c49ec29`. Later documentation or upload-tool
changes do not alter that source identity or replace the submitted build.

Package: `Yu-macOS-AppStore.pkg`; 45,926,548 bytes.

SHA256: `8a1a60f2684b7a4d35399148ac416457533e6dc0900123fa0840c92f0ceb17b0`.

The package, manifest, acceptance evidence, and redacted validation/upload logs
remain in `artifacts/appstore-submission-0.1.2-build3-ec726f99/`.

## Store submission and next release

The three store pages have descriptions, promotional text, keywords, support
links, privacy policy links, and localized 2880 × 1800 JPEG screenshots. Regional
availability and zero prices were saved for 175 countries/regions. Content
rights reflect the licenses shipped in the audited bundle. Export-compliance
answers were saved before submission.

Apple validation and upload succeeded. Warning 90889 identifies the missing
provisioning profile required for TestFlight; the App Store build was processed
and submitted successfully. TestFlight eligibility has not been established.

`tools/release-macos-appstore.py` builds and signs a fresh App Store package
from clean source using explicitly supplied signing identities and Team ID.
`tools/upload-macos-appstore.py` validates the manifest and package hash, then
performs Apple validation before upload. It reads an existing App-specific
password interactively, sends it through stdin, and redacts account credentials
from saved output. `--check-only` performs local verification without upload.

The store privacy policy links currently use the published release branch.
After merging, use the default branch URL for future metadata updates. The
policy stays publicly accessible in both branches.

Next external steps: Apple's review decision and manual release after approval.
Git tags and the joint GitHub Release remain deferred until Windows is ready.
