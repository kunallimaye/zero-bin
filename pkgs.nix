let
  rev = "8dee8bf3fd491cca2e6b541e17cd8bed4894dda6";
  sha = "1f02613346b8555qwypkr2jrx2vw18l3hfj3kr78rd0aff9sg3z3";

in (import (builtins.fetchTarball {
    name = "nixpkgs-pinned-${rev}";
    url = "https://github.com/NixOS/nixpkgs/archive/${rev}.tar.gz";
    sha256 = sha;
  }) {
    overlays = [
      (import (builtins.fetchTarball "https://github.com/oxalica/rust-overlay/archive/master.tar.gz"))
      (import ./overlay.nix)
    ];
})
