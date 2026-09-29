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

This branch contains an **incomplete** App Store sandbox path. The candidate
builder at `tools/release-macos-appstore.py` built and signed an Apple
Distribution app with separate app and helper entitlements, and
`codesign --verify --deep --strict` passed. A sandboxed empty-document window
self-check passed. External document open/save, relative image access, recent
files, and recovery still require full UI acceptance after merging the
internationalization work. The Open panel selected a dedicated `.md` test file
but left its Open button disabled; investigate this before upload. The team
also needs a Mac Installer Distribution certificate to sign the App Store
`.pkg`.

The app privacy policy is in `PRIVACY.md`; publish it on the default branch
before using its URL in App Store Connect. The repository README previously
called the app unusable; it now states the actual pre-release status. All
store answers must match the final signed build.

The existing Developer ID DMG pipeline and artifact are separate and were not
modified by this draft. The Mac App Store needs its own sandboxed package; the
notarized DMG is not an App Store build.
