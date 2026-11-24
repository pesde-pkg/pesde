{
  lib,
  rustPlatform,
  pkg-config,
  openssl,
  dbus,
}:
rustPlatform.buildRustPackage (_finalAttrs: {
  pname = "pesde";
  version = "0.7.1";

  src = ../.;

  cargoHash = "sha256-7rEU4vU6qp6eaj777cC3BgrRyiLRe+5xZC+cqV5TTc4=";

  buildNoDefaultFeatures = true;
  buildFeatures = [
    "bin"
    "version-management"
  ];

  nativeBuildInputs = [
    pkg-config
  ];
  buildInputs = [
    openssl
    dbus
  ];

  meta = {
    mainProgram = "pesde";
    description = "A package manager for the Luau programming language, supporting multiple runtimes including Roblox and Lune";
    homepage = "https://github.com/pesde-pkg/pesde";
    license = lib.licenses.mit;
    maintainers = [];
  };
})
