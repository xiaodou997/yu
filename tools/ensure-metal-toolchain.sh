#!/bin/sh
set -eu
if [ "$(uname -s)" = Darwin ]; then
    if ! xcrun metal --version >/dev/null 2>&1; then
        xcodebuild -downloadComponent MetalToolchain
    fi
    xcrun metal --version
fi
