# Every scenario, by name. Adding one: write nix/scenarios/<name>.nix (an
# attrset of name, description, repetitions and cells; see uuid-keys.nix) and
# list it here. `nix run .#scenarios.<name>` then runs it.
{
  pkgs,
  images,
}: {
  uuid-keys = import ./uuid-keys.nix {inherit pkgs images;};
}
