{
  description = "reagent: a resumable coding agent on subagent-net";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { nixpkgs, flake-utils, rust-overlay, ... }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; overlays = [ rust-overlay.overlays.default ]; };
        lib = pkgs.lib;
        # Releases are static musl binaries (Linux): `cargo build --release --target $REAGENT_MUSL_TARGET`.
        musl = "${pkgs.stdenv.hostPlatform.parsed.cpu.name}-unknown-linux-musl";
        muslEnv = lib.replaceStrings [ "-" ] [ "_" ] musl;
        staticCc = pkgs.pkgsStatic.stdenv.cc;
        rust = pkgs.rust-bin.stable.latest.default.override {
          extensions = [ "rust-src" "rust-analyzer" "clippy" "rustfmt" ];
          targets = lib.optional pkgs.stdenv.hostPlatform.isLinux musl;
        };
      in {
        devShells.default = pkgs.mkShell ({
          # git for worktrees, apprise for notifications, playwright for the UI tests.
          packages = [ rust pkgs.sqlx-cli pkgs.nodejs_22 pkgs.git pkgs.apprise pkgs.playwright-driver.browsers ];
          PLAYWRIGHT_BROWSERS_PATH = "${pkgs.playwright-driver.browsers}";
          PLAYWRIGHT_SKIP_VALIDATE_HOST_REQUIREMENTS = "true";
        } // lib.optionalAttrs pkgs.stdenv.hostPlatform.isLinux {
          # C dependencies (SQLite, the TLS crypto) built and linked for musl, statically.
          REAGENT_MUSL_TARGET = musl;
          "CC_${muslEnv}" = "${staticCc}/bin/${staticCc.targetPrefix}cc";
          "AR_${muslEnv}" = "${staticCc.bintools.bintools}/bin/${staticCc.targetPrefix}ar";
          "CARGO_TARGET_${lib.toUpper muslEnv}_LINKER" = "${staticCc}/bin/${staticCc.targetPrefix}cc";
          "CARGO_TARGET_${lib.toUpper muslEnv}_RUSTFLAGS" = "-C target-feature=+crt-static";
        });
      });
}
