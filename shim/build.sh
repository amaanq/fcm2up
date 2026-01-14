#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")"

echo "Building fcm2up-shim..."
./gradlew assembleRelease --no-daemon -q

echo "Converting to DEX..."
mkdir -p build/dex
cd build/dex
unzip -oq ../outputs/aar/fcm2up-shim-release.aar classes.jar
d8 --output . classes.jar 2>/dev/null || true

# Copy to output location
cp classes.dex ../../fcm2up-shim.dex

echo "Output: fcm2up-shim.dex ($(stat -c%s ../../fcm2up-shim.dex) bytes)"
