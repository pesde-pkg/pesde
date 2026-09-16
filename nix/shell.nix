{pkgs ? import <nixpkgs> {}, ...}: pkgs.mkShell {
  packages = with pkgs; [
    pkg-config
    openssl
    dbus
    cargo
    rustc
    clippy
    rust-analyzer
    (rustfmt.override {
      asNightly = true;
    })
  ];
}
