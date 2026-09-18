#!/bin/zsh
# Source from product builds: one SDK for Swift, Rust's C bridges and Metal.
set -euo pipefail
if [[ "$(uname -m)" != arm64 ]]; then
    print -u2 'Yu requires an Apple Silicon build host.'
    return 1
fi
yu_xcode_version="$(xcodebuild -version)"
yu_sdk_version="$(xcrun --sdk macosx --show-sdk-version)"
if [[ "$yu_xcode_version" != *'Build version 27A266a'* || "$yu_sdk_version" != 27.0 ]]; then
    print -u2 "Yu requires release Xcode 27 and macOS 27 SDK; found $yu_xcode_version / SDK $yu_sdk_version. Set DEVELOPER_DIR to the matching complete Xcode installation."
    return 1
fi
export SDKROOT="$(xcrun --sdk macosx --show-sdk-path)"
export MACOSX_DEPLOYMENT_TARGET=26.0
xcrun metal --version >/dev/null
xcrun --find metallib >/dev/null
