# Mac App Store draft (paused for internationalization)

The App Store Connect record for **Yu Markdown Editor** is saved as a draft:

- App Store Connect ID: `6817272771`
- Team ID: `V6M88BQG7C`
- Bundle ID: `io.github.xiaodou997.yu`
- macOS version: `0.1.2` (`Prepare for Submission`)
- Price: free (`$0.00` in the United States and other listed regions)
- Release after review: manual
- Review login: not required
- No build uploaded and no review submitted

The user is arranging macOS internationalization separately. Resume the store
listing, screenshots, regional availability, and submission after that work is
ready. The user will enter App Review contact details personally. Do not create
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

The existing Developer ID DMG pipeline and artifact are separate and were not
modified by this draft. The Mac App Store needs its own sandboxed package; the
notarized DMG is not an App Store build.
