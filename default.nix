{ pkgs ? import <nixpkgs> {} }:
let
  toonmux = pkgs.callPackage ./package.nix {};
in
pkgs.buildFHSEnv {
  name = "toonmux";
  targetPkgs = _: [
    toonmux
    pkgs.font-awesome
    pkgs.fontconfig
  ];
  runScript = "toonmux";
}
