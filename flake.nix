{
  description = "gpm — Android-first age-only gopass password client";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    systems.url = "github:nix-systems/default";
  };

  outputs =
    {
      nixpkgs,
      systems,
      ...
    }:

    let
      inherit (nixpkgs) lib;

      transposeAttrs =
        attrs:
        let
          keys = lib.attrNames attrs;
          subkeys = lib.attrNames (lib.head (lib.attrValues attrs));
        in
        lib.genAttrs subkeys (subkey: lib.genAttrs keys (key: attrs.${key}.${subkey}));

      forEachSupportedSystem = f: transposeAttrs (lib.genAttrs (import systems) f);
    in
    forEachSupportedSystem (
      system:

      let
        pkgs = nixpkgs.legacyPackages.${system};
      in
      {
        devShells.default = pkgs.mkShell {
          packages = with pkgs; [
            # rust toolchain
            cargo
            rustc
            rustfmt
            clippy
            rust-analyzer
            cargo-audit
            cargo-release
            cargo-outdated

            # node / frontend
            nodejs
            pnpm

            # tauri v2 (macOS uses system WebKit; no extra libs needed)
            pkg-config
            openssl

            # misc
            just
            jq
            prettier
            nixfmt
          ];
        };
      }
    );
}
