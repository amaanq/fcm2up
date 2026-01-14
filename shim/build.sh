#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")"

echo "Building fcm2up-shim (Kotlin)..."
./gradlew assembleRelease --no-daemon -q

echo "Converting to DEX..."
mkdir -p build/dex
cd build/dex

# Extract classes.jar from AAR
unzip -oq ../outputs/aar/fcm2up-shim-release.aar classes.jar

# Find kotlin-stdlib
KOTLIN_STDLIB=$(find ~/.gradle/caches -name "kotlin-stdlib-1.9*.jar" -not -name "*-sources*" 2>/dev/null | head -1)

if [ -n "$KOTLIN_STDLIB" ] && [ -f "$KOTLIN_STDLIB" ]; then
    echo "Extracting minimal kotlin runtime from: $KOTLIN_STDLIB"

    # Extract only the essential kotlin.jvm.internal classes
    mkdir -p kotlin_minimal
    cd kotlin_minimal
    unzip -oq "$KOTLIN_STDLIB" 'kotlin/jvm/internal/Intrinsics*.class' 'kotlin/jvm/internal/Ref*.class' 2>/dev/null || true
    cd ..

    # Create minimal kotlin jar
    if [ -d kotlin_minimal/kotlin ]; then
        jar cf kotlin-minimal.jar -C kotlin_minimal .
        echo "Created minimal kotlin runtime"
        d8 --output . classes.jar kotlin-minimal.jar 2>/dev/null || true
    else
        echo "Warning: Could not extract kotlin intrinsics"
        d8 --output . classes.jar 2>/dev/null || true
    fi
    rm -rf kotlin_minimal kotlin-minimal.jar
else
    echo "Warning: kotlin-stdlib not found, DEX may be incomplete"
    d8 --output . classes.jar 2>/dev/null || true
fi

# Copy to output location
cp classes.dex ../../fcm2up-shim.dex

echo "Output: fcm2up-shim.dex ($(stat -c%s ../../fcm2up-shim.dex) bytes)"
