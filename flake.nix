{
  description = "gpm — Android-first age-only gopass password client";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    systems.url = "github:nix-systems/default";
    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      nixpkgs,
      systems,
      fenix,
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
        pkgs = import nixpkgs {
          inherit system;
          config = {
            allowUnfree = true;
            android_sdk.accept_license = true;
          };
        };

        rustToolchain = fenix.packages.${system}.combine [
          fenix.packages.${system}.stable.toolchain
          fenix.packages.${system}.targets.aarch64-linux-android.stable.rust-std
          fenix.packages.${system}.targets.armv7-linux-androideabi.stable.rust-std
          fenix.packages.${system}.targets.x86_64-linux-android.stable.rust-std
          fenix.packages.${system}.targets.i686-linux-android.stable.rust-std
        ];

        androidEnv = pkgs.androidenv.override { licenseAccepted = true; };
        androidComp = androidEnv.composeAndroidPackages {
          cmdLineToolsVersion = "16.0";
          includeNDK = true;
          platformVersions = [
            "28"
            "35"
            "36"
          ];
          buildToolsVersions = [ "35.0.0" ];
          includeEmulator = false;
          includeSystemImages = false;
          cmakeVersions = [ "3.22.1" ];
        };

        ndkBin =
          "${androidComp.androidsdk}/libexec/android-sdk/ndk-bundle/toolchains/llvm/prebuilt/"
          + (if pkgs.stdenv.isDarwin then "darwin-x86_64" else "linux-x86_64")
          + "/bin";
      in
      {
        devShells.default = pkgs.mkShell {
          packages = with pkgs; [
            # Rust
            rustToolchain
            rust-analyzer
            cargo-audit
            cargo-release
            cargo-outdated

            # Frontend
            nodejs
            # workardoun: NixOS/nixpkgs#525627
            (pnpm.override { nodejs-slim = pkgs.nodejs-slim_latest; }) # TODO: remove after next bump of flake.lock

            # Android
            jdk17
            androidComp.androidsdk

            # Utils
            just
            jq
            nixfmt
            prettier
          ];

          ANDROID_HOME = "${androidComp.androidsdk}/libexec/android-sdk";
          ANDROID_SDK_ROOT = "${androidComp.androidsdk}/libexec/android-sdk";
          ANDROID_NDK_ROOT = "${androidComp.androidsdk}/libexec/android-sdk/ndk-bundle";

          # NDK toolchain for cross-compiling native C deps (OpenSSL, libgit2)
          # Fixes: rust-lang/rust#131407 — macOS ar creates corrupt Linux archives.
          # llvm-ar produces GNU-format archives that rustc can handle cross-platform.
          CC_aarch64_linux_android = "${ndkBin}/aarch64-linux-android28-clang";
          CC_armv7_linux_androideabi = "${ndkBin}/armv7a-linux-androideabi28-clang";
          CC_x86_64_linux_android = "${ndkBin}/x86_64-linux-android28-clang";
          CC_i686_linux_android = "${ndkBin}/i686-linux-android28-clang";

          # Use shellHook for PATH and AR/RANLIB — plain attr may be overridden by shell profile.
          # Both TARGET_AR and plain AR are set so openssl-sys's build script picks them up
          # regardless of which fallback it checks.
          # macOS-only: rust-lang/rust#131407 — macOS ar creates BSD-format archives
          # that rustc cannot handle when cross-compiling to Linux/Android targets.
          shellHook = ''
            export PATH="${ndkBin}:$PATH"
          ''
          + lib.optionalString pkgs.stdenv.isDarwin ''
            export AR="${ndkBin}/llvm-ar"
            export TARGET_AR="${ndkBin}/llvm-ar"
            export RANLIB="${ndkBin}/llvm-ranlib"
          '';
        };
      }
    );
}
