#!/usr/bin/env bash
# Downloads the original ActionsJS Data API Request step (1.7) and its templayed.js helper at a
# pinned commit into tests/.original/, as ES modules, for the two generate_*.mjs scripts.
set -euo pipefail
SHA=2c25c6a9d449abda23611c0f9b53d1680a340b18
cd "$(dirname "$0")"
base="https://raw.githubusercontent.com/Betty-Blocks-Services/Data-API-request-step/$SHA/functions"
mkdir -p .original/functions/dataApiRequest/1.7 .original/functions/utils
curl -fsSL "$base/dataApiRequest/1.7/index.js" \
  | sed 's#"../../utils/templayed"#"../../utils/templayed.mjs"#' \
  > .original/functions/dataApiRequest/1.7/index.mjs
curl -fsSL "$base/utils/templayed.js" > .original/functions/utils/templayed.mjs
echo "Fetched the original at $SHA into tests/.original/"
