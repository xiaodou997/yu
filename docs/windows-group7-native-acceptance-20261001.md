# Windows group 7 native acceptance, 2026-10-01

Status: **Windows x64 packaging implementation and local automated acceptance
passed. Production signing, installed MSIX acceptance, WACK, and Store submission
remain pending. Group 7 is not formally closed as a published Windows release.**

Host: Windows 11 Pro 24H2, 10.0.26100, x64, Intel Iris Xe, NTFS. Actual GUI tests
ran at 192 DPI. The single monitor's system scale was not switched in this round.
The SDK used by packaging is 10.0.22621.0. No Windows Store identity or available
code-signing certificate was found. Developer Mode and certificate trust settings
were not modified.

## Delivered implementation

- Native EXE resource build: existing Yu artwork, seven-size ICO, product/file
  version from Cargo, PMv2 DPI declaration, Common Controls v6, `asInvoker`, and
  long-path manifest. Large/small HWND class icons are also set and checked.
- MSIX desktop manifest: x64 native GUI entry point, five languages,
  `.md` / `.markdown` / `.mdown` associations, and `runFullTrust`. Development
  identity is visibly separate from mandatory Partner Center identity inputs.
- Locked Debug/Release packaging of shell and formula/Mermaid companion,
  scale/target-size PNG assets, PRI resource indexes, published Cargo notices,
  vendored renderer notices, and embedded font licenses.
- Release audit with source commit/dirty status, pipeline and Cargo.lock hashes,
  complete payload inventory, package SHA256, channel, version, and signing status.
- SDK MakeAppx schema validation and independent SDK unpack verification,
  fail-closed package/payload hash and identity checks, EXE PE/resource checks,
  and optional extracted-application/renderer smoke.
- Existing-certificate signing entry point for both executables and MSIX,
  Publisher matching, validity/private-key/EKU checks, HTTPS timestamp input,
  and signature verification. No private-key export or certificate import.
- Five-language listing drafts, separate Windows privacy policy, documented
  release commands, and Windows CI packaging/smoke/refusal gates.

The Windows version is `0.1.0.0` (EXE `0.1.0`), based on current Cargo version.
It does not reuse the macOS release's `0.1.2` number. The minimum declared OS is
10.0.19041.0; this round validates the Windows 11 host only.

## Executed validation

| Check | Result |
| --- | --- |
| Complete Rust workspace | 1,532 passed, 0 failed, 4 default ignored |
| Windows DirectWrite / D3D / shell unit checks | 20 / 4 / 22 passed |
| Explicit actual resource integration | 1 passed; PNG/SVG/math/Mermaid, D3D readback/recovery, composition model, Unicode save/reopen |
| External native/managed UI Automation client | 38 passed; includes actual HWND large/small icons, source/selection/tree/actions/events/geometry/lifetime |
| Release MSIX schema / unpack / inventory | Passed; 45 payload files, x64 PE32+, GUI subsystem, matching EXE/MSIX versions and manifest |
| Extracted Release shell | Actual HWND / rendering / close and external UIA passed, 192 DPI |
| Extracted Release companion | Math SVG 124 x 30, 6,338 UTF-8 bytes; Mermaid SVG 318 x 90, 1,400 bytes; request identity and clean shutdown passed |
| Packaging refusal/path checks | 13 passed: missing/development Store identity, Debug Store profile, HTTP timestamp, version mismatch, existing output, absent certificate, dirty Store checkout, package/payload hash drift, unexpected entry, mismatched identity, spaces/Unicode output path |
| Icon regeneration | 22 ICO/PNG outputs reproduce byte-identical hashes |
| Cargo license inventory | 371 external packages; 32 without published root notices remain explicitly listed; vendored and font notices included separately |
| Quality gates | Workspace all-targets clippy `-D warnings`, fmt, diff check passed |
| Dependency and CI parity | PowerShell audit of existing policy: 27 packages / 114 internal edges; all 16 CI run commands covered by `tools/verify.sh` |

The explicit resource integration test is one of the four default ignored tests
and was executed separately. The other three ignored cases were not executed.
This round does not claim new Python-tool execution: dependency and CI parity
audits used PowerShell against the existing policy/command definitions.

Embedding PMv2 also changed the unit-test process's DPI awareness. An existing
resource fixture's fixed physical 1000 x 1600 window became a smaller logical
viewport at 200%, culling its last diagram from a dark frame. The fixture now
scales that intended logical viewport by actual HWND DPI, and the complete
integration test passes without weakening its resource or GPU assertions.

An initial clippy run also caught build-script `unwrap` and Win32 integer-resource
pointer construction. Both were corrected; the final all-targets gate passed.

## Evidence and reproduction

Tracked entry points and commands are in [Windows packaging](windows-packaging.md).
Local ignored evidence includes:

- `artifacts/windows-group7-self-checks-final.log`
- `artifacts/windows-group7-workspace.log`
- `artifacts/windows-group7-clippy-pass.log`
- `artifacts/windows-group7-fmt-check.log`
- `artifacts/windows-group7/20261001/dependencies.log`
- `artifacts/windows-group7/release-verified/`: candidate release audit,
  MakeAppx/PRI logs, unpacked verification, UIA results, and 13 pipeline checks.
- `artifacts/windows-group7/clean-source/`: final development candidate generated
  from the committed clean source. Its `release-manifest.json` is authoritative
  for commit and package SHA256; `verification.json` records actual checks.

The release candidate MSIX is **unsigned**. It may be unpacked and inspected, but
is not a trusted installer. The extracted EXEs can be run together as a portable
development candidate. Do not present extracted-process smoke as installed-MSIX
acceptance.

## Outstanding release acceptance

Actual signed/development registration or Store installation, launch under package
identity, file association activation, update, uninstall, and a clean-machine
runtime check have not run. WACK is installed on this host but has not run against
an installed package. The signing code path is implemented with refusal checks;
a real successful production certificate/timestamp/signature test is pending.

Partner Center identity, publisher display name and reserved product name,
localized screenshots/translation review, markets/pricing, age rating, privacy
declarations, and restricted `runFullTrust` review remain pending. No package was
uploaded and no listing was published. Cross-compilation resource tooling and
Windows 10 compatibility are not validated by this native Windows 11 round.

The previous round's Narrator speech and actual system contrast-theme switching
also remain separate manual checks. They do not block constructing the package,
and are not included in this pass declaration.
