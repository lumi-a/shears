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
          pkgs.samply
          pkgs.libcap
        ];

        # The `samply` profiler needs CAP_PERFMON to call perf_event_open(), which
        # `kernel.perf_event_paranoid` (2 by default) denies to unprivileged users.
        # Rather than lowering that sysctl system-wide (which would affect every
        # process, not just this, and cause sidechannel insecurity risks or something),
        # we instead copy `samply` into a project-local, gitignored path and grant
        # the capability to only this one specific copy.
        # Will have to re-copy and re-approve if the nixpkgs pin changes `samply`'s
        # store path, oh well.
        shellHook = ''
          mkdir -p .samply-wrapped
          current_source="$(readlink -f ${pkgs.samply}/bin/samply)"
          cached_source="$(cat .samply-wrapped/.source 2>/dev/null || true)"
          if [ "$current_source" != "$cached_source" ]; then
            cp "$current_source" .samply-wrapped/samply
            echo "$current_source" > .samply-wrapped/.source
          fi
          if ! getcap .samply-wrapped/samply 2>/dev/null | grep -q cap_perfmon; then
            echo "Gotta grant cap_perfmon to .samply-wrapped/samply, needs sudo once."
            sudo setcap cap_perfmon+ep .samply-wrapped/samply
          fi
          export PATH="$PWD/.samply-wrapped:$PATH"
        '';
      };
    };
}
