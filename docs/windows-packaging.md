# Windows x64 release packaging

The Windows pipeline packages the native GUI shell and its local formula/Mermaid
helper together. It does not publish a GitHub Release, upload to Partner Center,
change certificate trust, enable Developer Mode, or register file associations.
Installer verification with `-Smoke` installs and uninstalls an isolated per-user
test copy; ordinary build/verification does not install it.

## Current public channel: unsigned GitHub EXE installer and portable ZIP

Windows distribution starts with unsigned GitHub Release EXE installers, with
portable ZIPs as an alternative. Microsoft Store
submission and paid signing are deferred until the product has sufficient users.
No Store identity, certificate, trust import, or MSIX installation is required
for this portable channel.

```powershell
./platform/windows/yu-shell-windows/build-package.ps1 -Channel GitHub -Smoke -TestPipeline
# Use the output directory printed by the first command:
./platform/windows/yu-shell-windows/build-installer.ps1 -ReleaseDirectory '<release directory>' -Smoke
```

The command builds `Yu-<Cargo-version>-windows-x64.zip` and `SHA256SUMS.txt` from a
clean Release checkout. The ZIP contains `Yu.exe`, the adjacent
`yu-document-renderer.exe`, license notices, and usage instructions. Users extract
the entire ZIP and run `Yu.exe`. Portable packaging does not generate an MSIX
manifest or PRI resources and does not invoke MakeAppx. It keeps the same embedded
EXE icon/version/DPI resources. `-Candidate` explicitly allows a dirty checkout
or Debug profile for local/CI checks; those builds are marked as candidates.

Verification opens the actual ZIP, checks entry paths/duplicates/sizes and payload
hashes, confirms both executables are unsigned, checks GUI/EXE resources, and runs
the independent UIA client plus formula/Mermaid companion from a fresh extracted
directory. The checksum file is checked against the release audit. It detects
changed downloads; it is not a publisher signature.

The second command requires Inno Setup (`ISCC.exe` on PATH, an installation in
Program Files, or `-CompilerPath` / `YU_INNO_COMPILER`). Inno Setup 7.1.0 was used
for local acceptance; CI can use its bundled Inno Setup 6 compiler. The script
independently verifies the ZIP input, stages the same executable/helper/license
payload with installer instructions and the Inno Setup license, then builds
`Yu-<Cargo-version>-windows-x64-setup.exe`. The resulting installer and its
uninstaller are unsigned. No certificate or signing service is required.

The installer defaults to `%LOCALAPPDATA%\Programs\Yu`, installs for the current
user without requesting elevation, creates a Start menu entry and Windows
Settings uninstall registration, and offers an optional desktop shortcut.
Users close Yu before reinstalling/updating. It does not install an updater or
change the default Markdown editor. Uninstall removes installed program files;
unknown user documents are retained, including documents inside the app folder.
English, Japanese and Korean installer translations are included; Simplified and
Traditional Chinese are also included when supplied by the compiler (both were
present in the locally verified Inno Setup 7.1.0 build). App UI languages remain
independent of the installer language set.

`installer-manifest.json` records source/version, compiler/script hashes, the ZIP
input hash, installer hash and installed payload inventory. `SHA256SUMS.txt`
lists both downloads. `verify-installer.ps1 -ReleaseDirectory '<directory>'
-Smoke` verifies the actual installer, runs an isolated current-user install,
checks payload hashes, uninstall registration and shortcut targets, launches
the installed HWND/UIA client and renderer, reinstalls the same version, then
uninstalls while checking that user documents are preserved. Evidence is written
to `installer-verification.json` and its referenced test directory. The test
refuses to replace an existing Yu installation. Same-version reinstall is not
cross-version upgrade acceptance. Actual Windows 10, other machines and
download/SmartScreen behavior require separate verification.

Users do not need admin rights or Developer Mode for the portable edition.
Windows SmartScreen can show an unknown-publisher/unrecognized-app warning;
Smart App Control and enterprise policies can prevent unsigned EXEs from running.
GitHub hosting does not sign the download or remove those restrictions. See
[Microsoft's SmartScreen guidance](https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/smartscreen-reputation).
The distribution does not change security settings, register file associations,
or install an updater. Users update by closing the app and extracting a new
release into another folder. Removing the extracted app folder removes the
portable distribution; documents remain at their chosen locations.

An unsigned MSIX is not the public GitHub installer. Windows 11's special unsigned
MSIX installation mechanism is intended for testing and is not used for this
release route. MSIX/Store packaging below remains an optional future channel.
Building the archive does not create a Git tag or publish a GitHub Release.

## Prerequisites and development build

Use Windows x64, Rust 1.98.1, and the Windows 10/11 SDK with `rc`, `mt`, `makepri`,
`makeappx`, and `signtool`. The resource build discovers SDK `rc.exe`; `RC` or
`LLVM_RC` can select a resource compiler explicitly. Non-Windows Cargo builds skip
Windows resources. Cross builds require LLVM `llvm-rc`; that path has not been
revalidated in this Windows acceptance round. Windows release resources currently
target x64 MSVC.

```powershell
./platform/windows/yu-shell-windows/build-package.ps1
./platform/windows/yu-shell-windows/verify-package.ps1 -ReleaseDirectory <output-directory> -Smoke
./platform/windows/yu-shell-windows/test-package.ps1 -ReleaseDirectory <output-directory>
```

Each build requires a new output directory, defaults to an ignored
`artifacts/windows-group7/<timestamp>-<id>/` directory, and builds locked Release
executables for `x86_64-pc-windows-msvc`. `-Profile Debug` is available for CI.
`-Smoke -TestPipeline` runs the extracted-app and refusal checks in the build
command; the Windows CI uses these options with the Debug profile.
Development identity defaults to `Yu.Editor.Development`, `CN=Yu Development`.
Those names are explicitly development placeholders, not Store-assigned identity.
Dirty checkout builds record both the source commit and full Git status as a
candidate. Package and EXE version follow Cargo: currently `0.1.0.0` / `0.1.0`.
The macOS product's `0.1.2` release number is not silently reused on Windows.

The payload includes EXE icon/version/manifest resources, PMv2 DPI awareness,
Common Controls v6, `asInvoker`, the render helper, scale/target-size logo assets,
PRI indexes, and license texts. Artwork derives from the existing macOS `Yu.png`;
`generate-assets.ps1` regenerates the checked-in ICO/PNG assets. The MSIX declares
five UI languages, one full-trust desktop application, and `.md`, `.markdown`,
`.mdown` file-type associations. It does not change the user's default editor.
Minimum Windows version is 10.0.19041.0; actual local acceptance uses 10.0.26100.0.

The license audit walks the locked, Windows-filtered shell/helper dependency
closure including build dependencies, excludes dev-only edges, copies published
root notices, and explicitly lists packages without published root notice files.
Vendored renderer licenses and bundled font notices are included separately.
This is an engineering inventory rather than legal approval.

## Verification and evidence

`release-manifest.json` records source identity, dirty status, version, channel,
signing status, package SHA256, and every payload file's size and SHA256.
`verification.json` records independent SDK unpack verification, the x64 PE32+
GUI check, EXE icon/version/DPI resources, manifest identity/languages/capabilities,
image sizes, and signature checks when signed. MakeAppx schema validation remains
enabled. Verification refuses package hash drift, missing/changed payload files,
unexpected package entries, and mismatched identities.

`-Smoke` launches the extracted application for a real HWND render check, runs
the independent 38-case UI Automation client (including HWND icons), and sends math and Mermaid jobs
to the extracted companion process. Those checks establish extracted payload
behavior. They do not establish behavior after MSIX installation. Installation,
Store certification, file associations, update, and uninstall remain separate
acceptance items.

## Signing and Store channel

Store builds require a clean checkout, Release profile, and exact Partner Center
identity. Missing identity fails before building. Development identity is refused.
Package versions use revision zero and must match the shell's Cargo version.

```powershell
./platform/windows/yu-shell-windows/build-package.ps1 -Channel Store `
  -IdentityName '<Partner Center package name>' `
  -Publisher '<Partner Center publisher subject>' `
  -PublisherDisplayName '<Partner Center publisher display name>'
```

For local certificate signing, pass `-CertificateThumbprint` for an existing
CurrentUser\My code-signing certificate, plus an HTTPS RFC3161 `-TimestampUrl`
for release signing. The certificate must hold a private key, be current, have
Code Signing EKU, and exactly match the manifest Publisher. Both executables and
the MSIX are signed and independently verified. The script neither exports
private keys nor installs certificates. It supports existing certificate-store
signing; PFX import and managed cloud signing are not implemented.

Microsoft Store signs packages during submission. Sideloaded MSIX packages require
a valid trusted signature, and certificate subject must match manifest Publisher.
An unsigned development candidate is suitable for payload inspection; it is not
a trusted installer. See Microsoft's [manual packaging guide](https://learn.microsoft.com/en-us/windows/msix/desktop/desktop-to-uwp-manual-conversion)
and [MSIX signing guide](https://learn.microsoft.com/en-us/windows/msix/package/sign-msix-package-guide).

`Store/listing-draft.json` contains English, Simplified/Traditional Chinese,
Japanese, and Korean listing drafts with support and Windows privacy links. It
does not claim macOS-only export/printing functionality. Store screenshots,
translation review, markets, pricing, age rating, restricted-capability review,
WACK, clean-machine installation, update, and uninstall must be completed before
submission. No Windows Store identity or production signing certificate was found
during this round, so no Store upload or signed installation is claimed.
