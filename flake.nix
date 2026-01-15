{
  description = "fcm2up - FCM to UnifiedPush relay framework";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { self, nixpkgs }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      forAllSystems = nixpkgs.lib.genAttrs systems;
    in
    {
      packages = forAllSystems (
        system:
        let
          pkgs = import nixpkgs {
            inherit system;
            config = {
              allowUnfree = true;
              android_sdk.accept_license = true;
            };
          };

          # Android SDK with build-tools
          androidComposition = pkgs.androidenv.composeAndroidPackages {
            platformVersions = [ "36" ];
            buildToolsVersions = [ "36.0.0" ];
            includeNDK = false;
          };

          androidSdk = androidComposition.androidsdk;

          # Fetch smali/baksmali JARs from Bitbucket
          smaliVersion = "2.5.2";
          baksmaliJar = pkgs.fetchurl {
            url = "https://bitbucket.org/JesusFreke/smali/downloads/baksmali-${smaliVersion}.jar";
            sha256 = "sha256-0xFiSMzk+C7Fox63+V7nXa/0Ld9u7Qq1c5c9xT+60uU=";
          };
          smaliJar = pkgs.fetchurl {
            url = "https://bitbucket.org/JesusFreke/smali/downloads/smali-${smaliVersion}.jar";
            sha256 = "sha256-lUQplXixb3cdiqjq7+DTcYygNHjBbzw1by/PE2a/sRY=";
          };

          # Script to patch kotlin intrinsics out of smali files
          patchKotlinIntrinsics = pkgs.writeShellScript "patch-kotlin-intrinsics" ''
            set -euo pipefail
            SMALI_DIR="$1"

            # Find all smali files and patch Kotlin stdlib references
            find "$SMALI_DIR" -name "*.smali" -exec sed -i \
              -e 's|Lkotlin/text/Charsets;->UTF_8:Ljava/nio/charset/Charset;|Ljava/nio/charset/StandardCharsets;->UTF_8:Ljava/nio/charset/Charset;|g' \
              -e 's|Lkotlin/jvm/internal/Intrinsics;->areEqual(Ljava/lang/Object;Ljava/lang/Object;)Z|Ljava/util/Objects;->equals(Ljava/lang/Object;Ljava/lang/Object;)Z|g' \
              {} +

            # Remove checkNotNull calls - need regex for register names
            for f in $(find "$SMALI_DIR" -name "*.smali"); do
              sed -i -E \
                -e 's|invoke-static \{v[0-9]+\}, Lkotlin/jvm/internal/Intrinsics;->checkNotNull\(Ljava/lang/Object;\)V|nop|g' \
                -e 's|invoke-static \{v[0-9]+, v[0-9]+\}, Lkotlin/jvm/internal/Intrinsics;->checkNotNull\(Ljava/lang/Object;Ljava/lang/String;\)V|nop|g' \
                -e 's|invoke-static \{v[0-9]+, v[0-9]+\}, Lkotlin/jvm/internal/Intrinsics;->checkNotNullExpressionValue\(Ljava/lang/Object;Ljava/lang/String;\)V|nop|g' \
                -e 's|invoke-static \{p[0-9]+\}, Lkotlin/jvm/internal/Intrinsics;->checkNotNull\(Ljava/lang/Object;\)V|nop|g' \
                -e 's|invoke-static \{p[0-9]+, v[0-9]+\}, Lkotlin/jvm/internal/Intrinsics;->checkNotNull\(Ljava/lang/Object;Ljava/lang/String;\)V|nop|g' \
                -e 's|invoke-static \{p[0-9]+, v[0-9]+\}, Lkotlin/jvm/internal/Intrinsics;->checkNotNullExpressionValue\(Ljava/lang/Object;Ljava/lang/String;\)V|nop|g' \
                "$f"
            done
          '';
        in
        {
          default = self.packages.${system}.fcm2up-bridge;

          # Bridge server
          fcm2up-bridge = pkgs.rustPlatform.buildRustPackage {
            pname = "fcm2up-bridge";
            version = "0.1.0";
            src = ./.;

            cargoLock.lockFile = ./Cargo.lock;
            buildAndTestSubdir = "bridge";

            nativeBuildInputs = [
              pkgs.pkg-config
              pkgs.protobuf
            ];
            buildInputs = [ pkgs.openssl ];

            meta = {
              description = "FCM to UnifiedPush relay server";
              mainProgram = "fcm2up-bridge";
            };
          };

          # Patcher CLI
          fcm2up-patcher = pkgs.rustPlatform.buildRustPackage {
            pname = "fcm2up-patcher";
            version = "0.1.0";
            src = ./.;

            cargoLock.lockFile = ./Cargo.lock;
            buildAndTestSubdir = "patcher";

            nativeBuildInputs = [ pkgs.pkg-config ];

            meta = {
              description = "APK patcher for FCM2UP";
              mainProgram = "fcm2up-patcher";
            };
          };

          # Shim DEX
          fcm2up-shim = pkgs.stdenv.mkDerivation (finalAttrs: {
            pname = "fcm2up-shim";
            version = "0.1.0";
            src = ./shim;

            nativeBuildInputs = [
              pkgs.gradle
              pkgs.jdk17
              pkgs.unzip
            ];

            # Pre-fetch Gradle dependencies (run `nix run .#update-shim-deps` to update)
            mitmCache = pkgs.gradle.fetchDeps {
              pkg = finalAttrs.finalPackage;
              data = ./shim/deps.json;
            };

            __darwinAllowLocalNetworking = true;

            ANDROID_SDK_ROOT = "${androidSdk}/libexec/android-sdk";

            gradleFlags = [
              "-Dorg.gradle.java.home=${pkgs.jdk17}"
              "-Dorg.gradle.project.android.aapt2FromMavenOverride=${androidSdk}/libexec/android-sdk/build-tools/36.0.0/aapt2"
            ];

            gradleBuildTask = "assembleRelease";
            gradleUpdateTask = "nixDownloadDeps";

            doCheck = false;

            preBuild = ''
              export JAVA_TOOL_OPTIONS="-Duser.home=$NIX_BUILD_TOP/home"
              mkdir -p $NIX_BUILD_TOP/home/.android
              echo "sdk.dir=$ANDROID_SDK_ROOT" > local.properties
            '';

            # After Gradle build, convert AAR to DEX and patch out Kotlin intrinsics
            postBuild = ''
              echo "Converting AAR to DEX..."
              mkdir -p build/dex
              cd build/dex

              # Extract classes.jar from AAR
              ${pkgs.unzip}/bin/unzip -q ../outputs/aar/fcm2up-shim-release.aar classes.jar

              # Convert to DEX using d8
              $ANDROID_SDK_ROOT/build-tools/36.0.0/d8 --release --output . classes.jar

              # Disassemble DEX to smali
              mkdir smali
              ${pkgs.jdk17}/bin/java -jar ${baksmaliJar} d classes.dex -o smali/

              # Patch out kotlin intrinsics
              ${patchKotlinIntrinsics} smali

              # Reassemble to DEX
              ${pkgs.jdk17}/bin/java -jar ${smaliJar} a smali/ -o fcm2up-shim.dex

              cd ../..
            '';

            installPhase = ''
              mkdir -p $out
              cp build/dex/fcm2up-shim.dex $out/
            '';

            meta = {
              description = "FCM2UP Kotlin shim compiled to DEX (kotlin-stdlib patched out)";
            };
          });
        }
      );

      nixosModules.default = import ./bridge/module.nix self;

      overlays.default = final: prev: {
        fcm2up-bridge = self.packages.${prev.system}.fcm2up-bridge;
        fcm2up-patcher = self.packages.${prev.system}.fcm2up-patcher;
        fcm2up-shim = self.packages.${prev.system}.fcm2up-shim;
      };

      devShells = forAllSystems (
        system:
        let
          pkgs = import nixpkgs {
            inherit system;
            config = {
              allowUnfree = true;
              android_sdk.accept_license = true;
            };
          };

          androidComposition = pkgs.androidenv.composeAndroidPackages {
            platformVersions = [ "36" ];
            buildToolsVersions = [ "36.0.0" ];
            includeNDK = false;
          };

          smaliVersion = "2.5.2";
          baksmaliJar = pkgs.fetchurl {
            url = "https://bitbucket.org/JesusFreke/smali/downloads/baksmali-${smaliVersion}.jar";
            sha256 = "sha256-0xFiSMzk+C7Fox63+V7nXa/0Ld9u7Qq1c5c9xT+60uU=";
          };
          smaliJar = pkgs.fetchurl {
            url = "https://bitbucket.org/JesusFreke/smali/downloads/smali-${smaliVersion}.jar";
            sha256 = "sha256-lUQplXixb3cdiqjq7+DTcYygNHjBbzw1by/PE2a/sRY=";
          };
        in
        {
          default = pkgs.mkShell {
            buildInputs = [
              # Rust toolchain
              pkgs.rustc
              pkgs.cargo
              pkgs.rust-analyzer
              pkgs.pkg-config
              pkgs.openssl
              pkgs.protobuf

              # Android/Java tooling
              pkgs.jdk17
              pkgs.gradle
              androidComposition.androidsdk

              # APK tools
              pkgs.apktool
              pkgs.apksigner
            ];

            ANDROID_SDK_ROOT = "${androidComposition.androidsdk}/libexec/android-sdk";
            JAVA_HOME = "${pkgs.jdk17}";
            OPENSSL_DIR = "${pkgs.openssl.dev}";
            OPENSSL_LIB_DIR = "${pkgs.openssl.out}/lib";
            PKG_CONFIG_PATH = "${pkgs.openssl.dev}/lib/pkgconfig";

            # Make smali tools available
            BAKSMALI_JAR = "${baksmaliJar}";
            SMALI_JAR = "${smaliJar}";

            shellHook = ''
              echo "fcm2up Development Shell"
              echo ""
              echo "Build commands:"
              echo "  nix build .#fcm2up-bridge   - Build bridge server"
              echo "  nix build .#fcm2up-patcher  - Build APK patcher"
              echo "  nix build .#fcm2up-shim     - Build shim DEX (with kotlin patching)"
              echo ""
              echo "Patch GitHub APK:"
              echo "  nix run .#patch-github         - Build shim + patch APK"
              echo "  nix run .#patch-github-install - Build, patch, and install"
              echo "  BRIDGE_URL=... nix run .#patch-github  - Override bridge URL"
              echo ""
              echo "Update shim deps (after changing shim/build.gradle.kts):"
              echo "  nix run .#update-shim-deps"
              echo ""
              echo "Manual tools:"
              echo "  java -jar \$BAKSMALI_JAR d <dex> -o <out>  - Disassemble DEX"
              echo "  java -jar \$SMALI_JAR a <dir> -o <dex>     - Assemble smali to DEX"
            '';
          };
        }
      );

      # App to update shim Gradle dependencies
      apps = forAllSystems (
        system:
        let
          pkgs = import nixpkgs {
            inherit system;
            config = {
              allowUnfree = true;
              android_sdk.accept_license = true;
            };
          };

          androidComposition = pkgs.androidenv.composeAndroidPackages {
            platformVersions = [ "36" ];
            buildToolsVersions = [ "36.0.0" ];
            includeNDK = false;
          };

          # Fetch smali/baksmali JARs
          smaliVersion = "2.5.2";
          baksmaliJar = pkgs.fetchurl {
            url = "https://bitbucket.org/JesusFreke/smali/downloads/baksmali-${smaliVersion}.jar";
            sha256 = "sha256-0xFiSMzk+C7Fox63+V7nXa/0Ld9u7Qq1c5c9xT+60uU=";
          };
          smaliJar = pkgs.fetchurl {
            url = "https://bitbucket.org/JesusFreke/smali/downloads/smali-${smaliVersion}.jar";
            sha256 = "sha256-lUQplXixb3cdiqjq7+DTcYygNHjBbzw1by/PE2a/sRY=";
          };

          # Wrapper scripts for baksmali/smali
          baksmaliWrapper = pkgs.writeShellScriptBin "baksmali" ''
            exec ${pkgs.jdk17}/bin/java -jar ${baksmaliJar} "$@"
          '';
          smaliWrapper = pkgs.writeShellScriptBin "smali" ''
            exec ${pkgs.jdk17}/bin/java -jar ${smaliJar} "$@"
          '';

          # Script to patch GitHub APK
          patchGithubScript = pkgs.writeShellScript "patch-github" ''
            set -euo pipefail

            BRIDGE_URL="''${BRIDGE_URL:-https://fcm-bridge.amaanq.com}"
            DISTRIBUTOR="''${DISTRIBUTOR:-io.heckel.ntfy}"
            GH_DIR="$HOME/projects/gh-android"
            INPUT_APK="$GH_DIR/apks/base.apk"
            OUTPUT_APK="$GH_DIR/github-fcm2up.apk"

            if [ ! -f "$INPUT_APK" ]; then
              echo "Error: Input APK not found at $INPUT_APK"
              echo "Pull it from your device with:"
              echo "  adb shell pm path com.github.android"
              echo "  adb pull <path>/base.apk $GH_DIR/apks/"
              exit 1
            fi

            echo "Building shim..."
            SHIM_DEX=$(nix build .#fcm2up-shim --no-link --print-out-paths)/fcm2up-shim.dex

            echo "Building patcher..."
            PATCHER=$(nix build .#fcm2up-patcher --no-link --print-out-paths)/bin/fcm2up-patcher

            echo "Patching APK..."
            echo "  Input:  $INPUT_APK"
            echo "  Output: $OUTPUT_APK"
            echo "  Bridge: $BRIDGE_URL"
            echo "  Distributor: $DISTRIBUTOR"

            # Set up environment for apksigner, apktool, baksmali, smali
            export PATH="${androidComposition.androidsdk}/libexec/android-sdk/build-tools/36.0.0:${pkgs.apktool}/bin:${baksmaliWrapper}/bin:${smaliWrapper}/bin:$PATH"

            "$PATCHER" patch \
              -i "$INPUT_APK" \
              -o "$OUTPUT_APK" \
              -b "$BRIDGE_URL" \
              -d "$DISTRIBUTOR" \
              --shim-dex "$SHIM_DEX"

            echo ""
            echo "Done! Patched APK: $OUTPUT_APK"
            echo ""
            echo "To install:"
            echo "  adb install -r $OUTPUT_APK"
            echo "Or for split APKs:"
            echo "  adb install-multiple $OUTPUT_APK $GH_DIR/apks/split_config.arm64_v8a.apk $GH_DIR/apks/split_config.xhdpi.apk"
          '';

          # Script to patch and install
          patchGithubInstallScript = pkgs.writeShellScript "patch-github-install" ''
            set -euo pipefail

            GH_DIR="$HOME/projects/gh-android"

            # Run the patch script
            ${patchGithubScript}

            echo "Installing..."
            ${pkgs.android-tools}/bin/adb install-multiple \
              "$GH_DIR/github-fcm2up.apk" \
              "$GH_DIR/apks/split_config.arm64_v8a.apk" \
              "$GH_DIR/apks/split_config.xhdpi.apk"

            echo ""
            echo "Installed! Check logs with:"
            echo "  adb logcat -s 'FCM2UP:*' 'Fcm2UpShim:*'"
          '';
        in
        {
          update-shim-deps = {
            type = "app";
            program = "${self.packages.${system}.fcm2up-shim.mitmCache.updateScript}";
          };

          # Patch GitHub APK (doesn't install)
          patch-github = {
            type = "app";
            program = "${patchGithubScript}";
          };

          # Patch and install GitHub APK
          patch-github-install = {
            type = "app";
            program = "${patchGithubInstallScript}";
          };
        }
      );
    };
}
