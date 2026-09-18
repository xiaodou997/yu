#!/bin/zsh
# Build the native macOS app. Options: --release. Apple Silicon only.
# stdout is the final app path; diagnostics go to stderr.
set -euo pipefail
shell_dir="${0:A:h}"
profile="debug"
source "$shell_dir/toolchain.sh"
for option in "$@"; do
    case "$option" in
        --release) profile="release" ;;
        *) print -r -- "Unknown build option: $option" >&2; exit 2 ;;
    esac
done
"$shell_dir/build-rust-ffi.sh" "$@" >&2
app_dir="$shell_dir/.build/Yu.app"
contents_dir="$app_dir/Contents"
mkdir -p "$contents_dir/MacOS" "$contents_dir/Resources"
swift build --package-path "$shell_dir" --triple arm64-apple-macosx26.0 -c "$profile" >&2
binary_dir="$(swift build --package-path "$shell_dir" --triple arm64-apple-macosx26.0 -c "$profile" --show-bin-path)"
# Replace the executable inode instead of overwriting a previously signed,
# possibly mapped image. In-place copies can leave stale code-signing pages
# that kill Mach-O inspection tools before the bundle is re-signed.
yu_staged_binary="$(mktemp "$contents_dir/MacOS/.Yu.XXXXXX")"
trap 'rm -f "$yu_staged_binary"' EXIT
cp -p "$binary_dir/Yu" "$yu_staged_binary"
mv -f "$yu_staged_binary" "$contents_dir/MacOS/Yu"
cp "$shell_dir/AppBundle/Info.plist" "$contents_dir/Info.plist"
cp -R "$shell_dir/AppBundle/Resources/." "$contents_dir/Resources/"
xcrun --sdk macosx metal -c -mmacosx-version-min=26.0 \
    "$shell_dir/../yu-render-macos/native/yu_shaders.metal" -o "$shell_dir/.rust/yu_shaders.air"
xcrun --sdk macosx metallib "$shell_dir/.rust/yu_shaders.air" \
    -o "$contents_dir/Resources/yu_shaders.metallib"

if ! yu_app_archs="$(lipo -archs "$contents_dir/MacOS/Yu")"; then
    print -u2 'Failed to inspect the staged Yu executable architecture.'
    exit 1
fi
if [[ "$yu_app_archs" != arm64 ]]; then
    print -u2 "Yu must be arm64 only; found: $yu_app_archs"
    exit 1
fi
xcrun vtool -show-build "$contents_dir/MacOS/Yu" >&2
codesign --force --sign - "$app_dir" >&2
python3 "$shell_dir/../../../tools/verify-macos-app.py" "$app_dir" \
    --configuration "$profile" --output "$shell_dir/.build/build-manifest.json" >&2
print -r -- "$app_dir"
