# Windows x64 release packaging

The Windows pipeline packages the native GUI shell and its local formula/Mermaid
helper together. It does not upload to Partner Center, change certificate trust,
enable Developer Mode, register file associations, or install a package.

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
