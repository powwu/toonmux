{ pkgs ? import <nixpkgs> {} }:
pkgs.mkShell {
  nativeBuildInputs = [ pkgs.pkg-config pkgs.rustc pkgs.cargo pkgs.clang pkgs.mold ];
  buildInputs = [ pkgs.gtk3 pkgs.xdotool pkgs.libX11 ];
}
