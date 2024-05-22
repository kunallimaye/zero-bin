{ pkgs, lib, env ? "local"}:

let
  is_local = (env == "local");
  is_ci = (env == "ci");


  jq-to-table = (pkgs.writeScriptBin "jq-to-table" ''
    ${pkgs.jq}/bin/jq -r '(map(keys) | add | unique) as $cols
      | map(. as $row | $cols | map($row[.])) as $rows | $cols, $rows[]
      | @csv' | ${pkgs.csvkit}/bin/csvlook
  '');

in with pkgs; {
  packages = [
    coreutils # GNU
    diffutils
    bash
  ] ++ lib.optionals (is_local || is_ci) [
    rust-bin.nightly.latest.default
    pkg-config
    openssl

    ## seems to compile without these -sys deps, until i know better lets just have them
    ## cargo tree | grep -i sys
    jemalloc
    zstd
    secp256k1

    gitleaks jq-to-table
    trivy
  ] ++ lib.optionals is_local [
    skopeo
  ];
}
