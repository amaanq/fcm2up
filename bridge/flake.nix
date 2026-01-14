{
  description = "fcm2up-bridge - FCM to UnifiedPush relay server";

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
          pkgs = nixpkgs.legacyPackages.${system};
        in
        {
          default = self.packages.${system}.fcm2up-bridge;
          fcm2up-bridge = pkgs.rustPlatform.buildRustPackage {
            pname = "fcm2up-bridge";
            version = "0.1.0";
            src = ./.;
            cargoLock.lockFile = ./Cargo.lock;

            nativeBuildInputs = [
              pkgs.pkg-config
              pkgs.protobuf
            ];
            buildInputs = [ pkgs.openssl ];

            meta = {
              description = "FCM to UnifiedPush relay server";
              homepage = "https://github.com/amaanq/fcm2up";
              license = pkgs.lib.licenses.mit;
              mainProgram = "fcm2up-bridge";
            };
          };
        }
      );

      nixosModules.default = import ./module.nix self;

      overlays.default = final: prev: {
        fcm2up-bridge = self.packages.${prev.system}.fcm2up-bridge;
      };

      devShells = forAllSystems (
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
        in
        {
          default = pkgs.mkShell {
            buildInputs = [
              pkgs.rustc
              pkgs.cargo
              pkgs.rust-analyzer
              pkgs.pkg-config
              pkgs.openssl
              pkgs.protobuf
            ];

            OPENSSL_DIR = "${pkgs.openssl.dev}";
            OPENSSL_LIB_DIR = "${pkgs.openssl.out}/lib";
            PKG_CONFIG_PATH = "${pkgs.openssl.dev}/lib/pkgconfig";
          };
        }
      );
    };
}
