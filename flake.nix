{
  description = "GitHub stacked-PR builder for those who miss Gerrit";

  inputs = {
    nixpkgs.url = "https://channels.nixos.org/nixpkgs-unstable/nixexprs.tar.xz";
    flake-parts.url = "github:hercules-ci/flake-parts";
  };

  outputs =
    inputs@{
      flake-parts,
      ...
    }:
    flake-parts.lib.mkFlake { inherit inputs; } {
      systems = inputs.nixpkgs.lib.systems.flakeExposed;
      perSystem =
        { pkgs, lib, ... }:
        let
          toml = (lib.importTOML ./Cargo.toml).package;
        in
        {
          packages = rec {
            gd = pkgs.rustPlatform.buildRustPackage (finalAttrs: {
              pname = toml.name;
              inherit (toml) version;
              cargoLock = {
                lockFile = ./Cargo.lock;
                allowBuiltinFetchGit = true;
              };
              src = ./.;
              nativeBuildInputs = [
                pkgs.installShellFiles
                pkgs.pkg-config
              ];
              buildInputs = [
                pkgs.openssl
              ];
              postInstall = ''
                installShellCompletion --cmd gd \
                  --bash gen/gd.bash \
                  --fish gen/gd.fish \
                  --zsh gen/_gd
                installManPage gen/*.1
              '';
            });
            default = gd;
          };
          formatter = pkgs.nixfmt-tree;
        };
    };
}
