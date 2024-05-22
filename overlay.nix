self: super:
{

  # kubernetes = super.kubernetes.overrideAttrs (old: rec {
  #   version = "1.25.12";
  #   name = "kubernetes-${version}";

  #   src = super.fetchFromGitHub {
  #     owner = "kubernetes";
  #     repo = "kubernetes";
  #     rev = "v${version}";
  #     hash = "sha256-eGvTzBNFevHi3qgqzdURH6jGUeKjwJxYfzPu+tGa294=";
  #   };
  # });

}
