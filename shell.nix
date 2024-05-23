{ env ? "local" }:

## import pinned package tree
with (import ./pkgs.nix);
let
  deps = pkgs.callPackage ./deps.nix { inherit pkgs env; };
in pkgs.mkShell {
  buildInputs = deps.packages;

  # RUST_BACKTRACE = 1;
  LD_LIBRARY_PATH = lib.makeLibraryPath [
    pkgs.openssl

    ## not sure if needed, maybe tests just dont cover them
    pkgs.jemalloc pkgs.zstd pkgs.secp256k1
  ];
}
