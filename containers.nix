with (import ./pkgs.nix);
let
  ci-deps = pkgs.callPackage ./deps.nix { inherit pkgs; env = "ci"; };

in rec {
  base = pkgs.dockerTools.buildLayeredImage {
    name = "base";
    tag = "latest";

    extraCommands = ''
      mkdir -m 1777 tmp
      mkdir -p usr/bin run bin lib64
      ln -s ${pkgs.coreutils}/bin/env usr/bin/env

      # github-actions
      ln -s ${pkgs.glibc}/lib/ld-linux-x86-64.so.2 lib64/ld-linux-x86-64.so.2
    '';

    contents = [
      pkgs.bash pkgs.coreutils pkgs.gnugrep
      pkgs.jq
      pkgs.curl
      pkgs.cacert

      # github-actions
      pkgs.gnutar pkgs.gzip
    ];

    config = {
      Env = [
        "SSL_CERT_FILE=/etc/ssl/certs/ca-bundle.crt"
        "LC_ALL=C.UTF-8" "LANG=C.UTF-8"

        # github-actions
        "LD_LIBRARY_PATH=${lib.makeLibraryPath [ pkgs.stdenv.cc.cc ]}"
      ];
    };
  };


  ci-tools = pkgs.dockerTools.buildLayeredImage {
    name = "ci-tools";
    tag = "latest";
    fromImage = base;
    contents = ci-deps.packages;

    # we should be able to extend from base image, but no time for that right now
    config = {
      Env = [
        "SSL_CERT_FILE=/etc/ssl/certs/ca-bundle.crt"
        "LC_ALL=C.UTF-8" "LANG=C.UTF-8"

        "LD_LIBRARY_PATH=${lib.makeLibraryPath [
          pkgs.stdenv.cc.cc # github-actions

          pkgs.openssl
          ## not sure if needed, maybe tests just dont cover them
          pkgs.jemalloc pkgs.zstd pkgs.secp256k1
        ]}"
      ];
    };
  };
}
