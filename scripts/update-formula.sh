#!/usr/bin/env bash
#
# Update Homebrew formula SHA256 values after a GitHub release.
#
# Usage:
#   ./scripts/update-formula.sh v0.6.6
#
# Prerequisites:
#   - gh CLI authenticated
#   - Release assets already uploaded (run after release workflow completes)

set -euo pipefail

VERSION="${1:?Usage: $0 <version-tag>  (e.g. v0.6.6)}"
REPO="pastel-sketchbook/melos-rs"
FORMULA_DIR="Formula"

echo "Downloading SHA256 files for ${VERSION}..."

for binary in melos-rs melos-tui; do
  for target in aarch64-apple-darwin x86_64-apple-darwin aarch64-unknown-linux-gnu x86_64-unknown-linux-gnu; do
    asset="${binary}-${VERSION}-${target}.tar.gz.sha256"
    echo "  ${asset}"
    gh release download "${VERSION}" --repo "${REPO}" --pattern "${asset}" --dir /tmp --clobber
  done
done

echo ""
echo "Updating formulas..."

update_formula() {
  local formula="$1"
  local binary="$2"

  local arm64_mac=$(awk '{print $1}' "/tmp/${binary}-${VERSION}-aarch64-apple-darwin.tar.gz.sha256")
  local x86_mac=$(awk '{print $1}' "/tmp/${binary}-${VERSION}-x86_64-apple-darwin.tar.gz.sha256")
  local arm64_linux=$(awk '{print $1}' "/tmp/${binary}-${VERSION}-aarch64-unknown-linux-gnu.tar.gz.sha256")
  local x86_linux=$(awk '{print $1}' "/tmp/${binary}-${VERSION}-x86_64-unknown-linux-gnu.tar.gz.sha256")

  local ver="${VERSION#v}"  # strip leading v

  sed -i"" \
    -e "s/version \".*\"/version \"${ver}\"/" \
    "${formula}"

  # Update SHA256 values in order of appearance (arm mac, intel mac, arm linux, intel linux)
  python3 -c "
import re, sys

with open('${formula}', 'r') as f:
    content = f.read()

shas = ['${arm64_mac}', '${x86_mac}', '${arm64_linux}', '${x86_linux}']
idx = 0

def replace_sha(match):
    global idx
    if idx < len(shas):
        result = f'sha256 \"{shas[idx]}\"'
        idx += 1
        return result
    return match.group(0)

content = re.sub(r'sha256 \"[A-Za-z0-9_]+\"', replace_sha, content)

with open('${formula}', 'w') as f:
    f.write(content)
"

  echo "  Updated ${formula} to ${ver}"
}

update_formula "${FORMULA_DIR}/melos-rs.rb" "melos-rs"
update_formula "${FORMULA_DIR}/melos-tui.rb" "melos-tui"

echo ""
echo "Done! Review changes with: git diff Formula/"
echo "Then commit and push to update the tap."
