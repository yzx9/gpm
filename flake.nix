# SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
#
# SPDX-License-Identifier: Apache-2.0

{
  description = "gpm — Android-first age-only gopass password client";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    systems.url = "github:nix-systems/default";
    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    git-hooks = {
      url = "github:cachix/git-hooks.nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      nixpkgs,
      systems,
      fenix,
      git-hooks,
      ...
    }@inputs:

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
    forEachSupportedSystem (system: {
      devShells = import ./nix/devShells.nix {
        inherit inputs system;
      };
    })
    // {
      # fenix (the Rust toolchain input) publishes its prebuilt artifacts to its
      # own cachix cache (fenix.cachix.org), not cache.nixos.org. Without this
      # substituter, devShells cold-build the entire Rust toolchain from source.
      nixConfig = {
        extra-substituters = [ "https://fenix.cachix.org" ];
        extra-trusted-public-keys = [
          "fenix.cachix.org-1:ecJhr+RdYEdcVgUkjruiYhjbBloIEGov7bos90cZi0Q="
        ];
      };
    };
}
