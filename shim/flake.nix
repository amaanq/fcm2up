{
  description = "fcm2up-shim - Kotlin library for FCM-to-UnifiedPush bridging";

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
            platformVersions = [ "34" ];
            buildToolsVersions = [ "34.0.0" ];
            includeNDK = false;
          };
        in
        {
          default = pkgs.mkShell {
            buildInputs = [
              androidComposition.androidsdk
              pkgs.jdk17
              pkgs.kotlin
              pkgs.gradle
            ];

            ANDROID_SDK_ROOT = "${androidComposition.androidsdk}/libexec/android-sdk";

            shellHook = ''
              export JAVA_HOME=${pkgs.jdk17}
              export GRADLE_USER_HOME=$(pwd)/.gradle

              echo "sdk.dir=$ANDROID_SDK_ROOT" > local.properties

              echo "fcm2up-shim Development Shell"
              echo "Build DEX: ./gradlew assembleRelease"
              echo "Output: build/outputs/aar/"
            '';
          };
        }
      );
    };
}
