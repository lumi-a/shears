{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    { nixpkgs, rust-overlay, ... }:
    let
      system = "x86_64-linux";
      pkgs = import nixpkgs {
        inherit system;
        overlays = [ rust-overlay.overlays.default ];
      };
      toolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;

      # rustfmt.toml uses nightly-only options.
      # Everything else stays on the stable toolchain.
      nightlyRustfmtFull = pkgs.rust-bin.nightly.latest.minimal.override {
        extensions = [ "rustfmt" ];
      };
      nightlyRustfmt = pkgs.runCommand "nightly-rustfmt-only" { } ''
        mkdir -p $out/bin
        ln -s ${nightlyRustfmtFull}/bin/rustfmt $out/bin/rustfmt
        ln -s ${nightlyRustfmtFull}/bin/cargo-fmt $out/bin/cargo-fmt
      '';
    in
    {
      devShells.x86_64-linux.default = pkgs.mkShell {
        buildInputs = [
          nightlyRustfmt
          toolchain
          pkgs.rust-analyzer
        ];
      };
    };
}
