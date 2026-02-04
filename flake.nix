{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-25.11";
    crane.url = "github:ipetkov/crane";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs =
    {
      self,
      nixpkgs,
      crane,
      flake-utils,
      ...
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs {
          inherit system;
        };

        craneLib = crane.mkLib pkgs;

        gitRev = self.rev or self.dirtyRev or null;

        prometheus-sd-nexthop = craneLib.buildPackage {
          src = craneLib.cleanCargoSource ./.;

          env.PROMETHEUS_SD_NEXTHOP_NIX_BUILD_REV = gitRev;
        };
      in
      {
        packages.default = prometheus-sd-nexthop;

        checks.prometheus-sd-nexthop = pkgs.testers.runNixOSTest (
          import ./test.nix { inherit prometheus-sd-nexthop gitRev; }
        );
      }
    );
}
