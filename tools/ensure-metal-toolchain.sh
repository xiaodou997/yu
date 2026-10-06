#!/bin/sh
set -eu
if [ "$(uname -s)" = Darwin ]; then
    if ! xcrun metal --version >/dev/null 2>&1; then
        xcodebuild -downloadComponent MetalToolchain
        xcrun --kill-cache
        # The asset download can finish before macOS mounts the toolchain.
        attempt=0
        until xcrun metal --version >/dev/null 2>&1; do
            attempt=$((attempt + 1))
            if [ "$attempt" -ge 12 ]; then
                echo 'Downloaded Metal toolchain is still unavailable.' >&2
                exit 1
            fi
            sleep 2
            xcrun --kill-cache
        done
    fi
    xcrun metal --version
fi
