{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-25.05";
    cargo2nix.url = "github:cargo2nix/cargo2nix/release-0.12";
    flake-utils.follows = "cargo2nix/flake-utils";
    # nixpkgs.follows = "cargo2nix/nixpkgs";
  };

  outputs =
    inputs:
    with inputs;
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ cargo2nix.overlays.default ];
        };

        rustPkgs = pkgs.rustBuilder.makePackageSet {
          rustVersion = "latest";
          packageFun = import ./Cargo.nix;
        };

      in
      rec {
        packages = {
          prometheus-sd-nexthop = (rustPkgs.workspace.prometheus-sd-nexthop { });
          default = packages.prometheus-sd-nexthop;
        };

        checks.prometheus-sd-nexthop = pkgs.testers.runNixOSTest (import ./test.nix { inherit packages; });
      }
    );
}
