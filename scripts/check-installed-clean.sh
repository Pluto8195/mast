#!/usr/bin/env bash
# Guard the agent-only driver boundary before writing a release-test stamp.
set -euo pipefail
cd "$(dirname "$0")/.."

# Cargo tree has no --profile option. Normal and build edges, across targets,
# describe the installed build without including development dependencies.
# After reviewing an intentional graph change, regenerate the baseline with:
# cargo tree -p mast --target all -e normal,build -f '{p} [{f}]' --locked |
#   python3 -c 'import os,sys; sys.stdout.write(sys.stdin.read().replace(os.getcwd(), "<workspace>"))' > scripts/installed-dependency-tree.txt
tree_file="$(mktemp "${TMPDIR:-/tmp}/mast-installed-tree.XXXXXX")"
symbol_file="$(mktemp "${TMPDIR:-/tmp}/mast-installed-symbols.XXXXXX")"
trap 'rm -f "$tree_file" "$symbol_file"' EXIT
cargo metadata --format-version 1 --no-deps --locked | python3 -c '
import json, sys
packages = json.load(sys.stdin)["packages"]
for package in packages:
    if package["name"] in ("mast", "mast-ui"):
        for dependency in package["dependencies"]:
            if dependency["name"] == "mast-ui-driver" and dependency["kind"] != "dev":
                sys.exit(package["name"] + " has a non-dev dependency on mast-ui-driver")
'
cargo tree -p mast --target all -e normal,build -f '{p} [{f}]' --locked |
  python3 -c 'import os,sys; sys.stdout.write(sys.stdin.read().replace(os.getcwd(), "<workspace>"))' > "$tree_file"
if grep -E 'mast-ui-driver v|\[([^]]*,)?(snapshot|test-support|ui-driver)(,|])' "$tree_file"; then
  echo 'installed dependency graph contains driver or test features' >&2
  exit 1
fi
if ! diff -u scripts/installed-dependency-tree.txt "$tree_file"; then
  echo 'installed dependency graph changed; review and update the baseline deliberately' >&2
  exit 1
fi

# Fail closed if the host refuses to lower the build priority.
python3 -c 'import os; os.nice(10); os.execvp("cargo", ["cargo", "build", "--profile", "installed", "-p", "mast", "--bin", "mast", "--locked"])'
binary=target/installed/mast
nm "$binary" > "$symbol_file"
strings "$binary" >> "$symbol_file"
if grep -iE 'mast[_-]ui[_-]driver|cue[_-]harness|world[_-]generator|ui[_-]drive' "$symbol_file"; then
  echo 'installed binary contains driver symbols or strings' >&2
  exit 1
fi
echo 'installed dependency graph and binary are clean'
