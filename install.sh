#!/usr/bin/env sh
# game-project-graph installer (Linux / macOS / WSL)
#
# Builds the Rust binary globally with `cargo install` and optionally registers
# the MCP server.  One global install serves every project.
set -eu

SCOPE="global"
CLIENTS="codex,cursor,vscode,air,cowork,workbuddy,project"
DO_MCP=1

while [ $# -gt 0 ]; do
  case "$1" in
    --scope) SCOPE="$2"; shift 2 ;;
    --clients) CLIENTS="$2"; shift 2 ;;
    --no-mcp) DO_MCP=0; shift ;;
    -h|--help) sed -n '2,10p' "$0"; exit 0 ;;
    *) echo "unknown option: $1" >&2; exit 2 ;;
  esac
done

if ! command -v cargo >/dev/null 2>&1; then
  echo "ERROR: cargo not found. Install Rust first: https://rustup.rs/" >&2
  exit 1
fi
echo "Rust: $(rustc --version)"

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
echo "Installing codegraph233 from $ROOT ..."
cargo install --path "$ROOT" --force

CARGO_BIN="${CARGO_HOME:-$HOME/.cargo}/bin"
case ":$PATH:" in
  *":$CARGO_BIN:"*) ;;
  *) echo "NOTE: add $CARGO_BIN to PATH" >&2 ;;
esac
export PATH="$CARGO_BIN:$PATH"
codegraph233 version

if [ "$DO_MCP" -eq 1 ] && [ -f ".codegraph233.json" ]; then
  echo "Registering MCP server for: $CLIENTS (scope=$SCOPE)"
  codegraph233 install-mcp --clients "$CLIENTS" --exe "codegraph233" --scope "$SCOPE" || true
fi

echo
echo "Done. Next steps:"
echo "  codegraph233 init      # write .codegraph233.json"
echo "  codegraph233 index     # build/update the index"
echo "  codegraph233 status    # show counts"
echo "  codegraph233 serve     # MCP stdio (clients start it automatically)"`necho "  codegraph233 update --check  # update check (also runs automatically)"
