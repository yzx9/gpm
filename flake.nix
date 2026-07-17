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
      self,
      nixpkgs,
      systems,
      fenix,
      git-hooks,
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

        # Full Rust toolchain: host stable + the four Android targets.
        rustToolchain = fenix.packages.${system}.combine [
          fenix.packages.${system}.stable.toolchain
          fenix.packages.${system}.targets.aarch64-linux-android.stable.rust-std
          fenix.packages.${system}.targets.armv7-linux-androideabi.stable.rust-std
          fenix.packages.${system}.targets.x86_64-linux-android.stable.rust-std
          fenix.packages.${system}.targets.i686-linux-android.stable.rust-std
        ];

        # Host-only stable toolchain (no Android targets) for the lighter shells.
        hostRustToolchain = fenix.packages.${system}.stable.toolchain;

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

        # Generated / volatile paths the write-hooks must not touch. prettier also
        # honors .prettierignore; this covers the whitespace hooks and mirrors it.
        formatExcludes = [
          "^src-tauri/gen/android/"
          "^pnpm-lock\\.yaml$"
          "^Cargo\\.lock$"
          "^rustpass/data/cacert\\.pem$"
          "^dist/"
          "^\\.agents/skills/"
        ];

        # git pre-commit hooks, auto-installed into the devShell (direnv sets
        # core.hooksPath via pre-commit-checks.shellHook). Unlike the old
        # smart_format.sh PostToolUse hook, this fires for *every* write path
        # (shell / heredoc / git), closing the formatter-bypass gap.
        pre-commit-checks = git-hooks.lib.${system}.run {
          src = ./.;
          hooks = {
            nixfmt.enable = true;

            prettier = {
              enable = true;
              excludes = formatExcludes;
            };

            # Per-file rustfmt (edition 2024) — formats only staged .rs files.
            # `cargo fmt` would reformat the whole workspace and drag unrelated
            # files into the commit, so we call rustfmt directly on staged files.
            rustfmt = {
              enable = true;
              name = "rustfmt";
              description = "Format staged Rust files (edition 2024)";
              entry = "rustfmt --edition 2024";
              files = "\\.rs$";
              language = "system";
              # git-hooks.nix defaults pass_filenames to false for custom hooks;
              # rustfmt must receive the staged filenames to format per-file.
              pass_filenames = true;
            };
          };
        };

        # Tauri desktop host stack. mkShell has no $out, so nix's cc/ld wrapper
        # stamps the gpm test binary with a dead RUNPATH (outputs/out/lib);
        # makeLibraryPath puts these /lib dirs on LD_LIBRARY_PATH so the linker
        # finds libgdk-3/libgtk-3 at runtime. The list tracks the packages, so
        # nix roll-forwards need no manual store-hash bumps.
        linuxDesktopRuntime = with pkgs; [
          glib
          gtk3
          cairo
          pango
          gdk-pixbuf
          atk
          webkitgtk_4_1
          libsoup_3
          dbus
        ];

        # shellHook fragment: the desktop runtime on LD_LIBRARY_PATH (see above).
        # Shared by `full` and `lite` — the two shells that link Tauri for the host.
        linuxDesktopLdHook = lib.optionalString pkgs.stdenv.isLinux ''
          export LD_LIBRARY_PATH="${lib.makeLibraryPath linuxDesktopRuntime}:$LD_LIBRARY_PATH"
        '';
      in
      {
        devShells = {
          default = self.devShells.${system}.full;

          # `full`: every toolchain the repo touches — the four Android Rust
          # targets, the Android SDK/NDK, JDK, and the desktop runtime. Used by CD
          # and test-android (the Android builds); also what `default` (bare
          # `nix develop`) resolves to for local development.
          full = pkgs.mkShell {
            packages =
              with pkgs;
              [
                # Rust
                rustToolchain
                rust-analyzer
                cargo-audit
                cargo-release
                cargo-outdated
                sccache # shared compile cache across worktrees (RUSTC_WRAPPER below)

                # Frontend
                nodejs
                pnpm

                # Android
                jdk17
                androidComp.androidsdk

                # Utils
                just
                jq
                nixfmt
                prettier

                # Cross-tool crypto interop: decrypt a gpm-created .age with the bare
                # `age` CLI (independent of rustpass's own decrypt path).
                age

                # Cross-tool store interop: drive the real `gopass` binary (age backend)
                # so the gopass-interop tests verify gpm reads a store gopass produced.
                gopass
              ]
              ++ lib.optionals pkgs.stdenv.isLinux (
                [
                  pkg-config # pkg-config is build-time only
                ]
                ++ linuxDesktopRuntime
              );

            ANDROID_HOME = "${androidComp.androidsdk}/libexec/android-sdk";
            ANDROID_SDK_ROOT = "${androidComp.androidsdk}/libexec/android-sdk";
            ANDROID_NDK_ROOT = "${androidComp.androidsdk}/libexec/android-sdk/ndk-bundle";
            # Tauri's Android build reads NDK_HOME (not just ANDROID_NDK_ROOT) and
            # JAVA_HOME; missing either, `tauri android build` aborts early.
            NDK_HOME = "${androidComp.androidsdk}/libexec/android-sdk/ndk-bundle";
            JAVA_HOME = "${pkgs.jdk17}/lib/openjdk";
            # AGP downloads a generic FHS aapt2 from Maven that the Nix stub-ld
            # refuses to run; point it at the Nix SDK's patchelf'd aapt2 instead.
            # Keep this build-tools version in sync with buildToolsVersions above.
            GRADLE_OPTS = "-Dorg.gradle.project.android.aapt2FromMavenOverride=${androidComp.androidsdk}/libexec/android-sdk/build-tools/35.0.0/aapt2";

            # NDK toolchain for cross-compiling native C deps (OpenSSL, libgit2)
            # Fixes: rust-lang/rust#131407 — macOS ar creates corrupt Linux archives.
            # llvm-ar produces GNU-format archives that rustc can handle cross-platform.
            CC_aarch64_linux_android = "${ndkBin}/aarch64-linux-android28-clang";
            CC_armv7_linux_androideabi = "${ndkBin}/armv7a-linux-androideabi28-clang";
            CC_x86_64_linux_android = "${ndkBin}/x86_64-linux-android28-clang";
            CC_i686_linux_android = "${ndkBin}/i686-linux-android28-clang";

            # sccache wraps rustc; its cache is machine-global, so a fresh worktree reuses
            # compiles from other worktrees instead of rebuilding target/ from scratch.
            # For max cold-build hits, run that build with CARGO_INCREMENTAL=0 (don't set it
            # here — it would slow `just dev` warm rebuilds). rust-analyzer ignores this.
            RUSTC_WRAPPER = "sccache";

            # Use shellHook for PATH and AR/RANLIB — plain attr may be overridden by shell profile.
            # Both TARGET_AR and plain AR are set so openssl-sys's build script picks them up
            # regardless of which fallback it checks.
            # macOS-only: rust-lang/rust#131407 — macOS ar creates BSD-format archives
            # that rustc cannot handle when cross-compiling to Linux/Android targets.
            shellHook =
              pre-commit-checks.shellHook
              + ''
                export PATH="${ndkBin}:$PATH"
              ''
              + linuxDesktopLdHook
              + lib.optionalString pkgs.stdenv.isDarwin ''
                export AR="${ndkBin}/llvm-ar"
                export TARGET_AR="${ndkBin}/llvm-ar"
                export RANLIB="${ndkBin}/llvm-ranlib"
              '';
          };

          # `lite`: host Rust + Tauri desktop runtime + frontend/tools, but NO
          # Android SDK/NDK/JDK/android targets. For lint, test (be/fe) and
          # format-fe — everything that compiles/links Tauri for the host or runs
          # the frontend, but never cross-compiles to Android.
          lite = pkgs.mkShell {
            packages =
              with pkgs;
              [
                hostRustToolchain
                sccache
                nodejs
                pnpm
                just
                nixfmt
                prettier
                age
              ]
              ++ lib.optionals pkgs.stdenv.isLinux (
                [
                  pkg-config
                ]
                ++ linuxDesktopRuntime
              );

            RUSTC_WRAPPER = "sccache";

            # CI-only shell (local dev uses `default`/`full`): skip the pre-commit
            # hook install, just expose the desktop runtime for linking Tauri.
            shellHook = linuxDesktopLdHook;
          };
        };
      }
    )
    // {
      # fenix (Rust toolchain) and git-hooks.nix publish prebuilt artifacts to
      # the nix-community binary cache, not cache.nixos.org. Without this
      # substituter, devShells cold-build the Rust toolchain from source.
      nixConfig = {
        extra-substituters = [ "https://fenix.cachix.org" ];
        extra-trusted-public-keys = [
          "fenix.cachix.org-1:ecJhr+RdYEdcVgUkjruiYhjbBloIEGov7bos90cZi0Q="
        ];
      };
    };
}
