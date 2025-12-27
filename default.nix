let
  lock = builtins.fromJSON (builtins.readFile ./flake.lock);
  flakeCompat = fetchTarball {
    url = "https://github.com/edolstra/flake-compat/archive/${lock.nodes.flake-compat.locked.rev}.tar.gz";
    sha256 = lock.nodes.flake-compat.locked.narHash;
  };
  pesde = import flakeCompat {
    src = ./.;
  };
in
  pesde.defaultNix
