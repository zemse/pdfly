#!/usr/bin/env bash
# Re-clone the pinned reference sources into reference/ (gitignored).
# Line numbers in ARCHITECTURE.md match these commits.
set -euo pipefail
cd "$(dirname "$0")/.."
mkdir -p reference
clone() { # repo url, dir, sha
  [ -d "reference/$2" ] && { echo "$2 present"; return; }
  git clone "$1" "reference/$2"
  git -C "reference/$2" checkout -q "$3" || echo "WARN: could not checkout $3 (shallow?)"
}
clone https://github.com/opendataloader-project/opendataloader-pdf opendataloader-pdf 15450c28e417b36634d33be3d0087b6419f9e862
clone https://github.com/veraPDF/veraPDF-wcag-algs           veraPDF-wcag-algs   4f0808b3d0324fdd8668a08ddc5a5a190774dc3d
clone https://github.com/veraPDF/veraPDF-parser              veraPDF-parser      862b1f227a184fefeafd959f1e0346b8fde1f9fa
echo "references ready"
