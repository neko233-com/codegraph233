#!/usr/bin/env sh
# codegraph233 installer (Linux / macOS / WSL)
#
# Usage:
#   ./install.sh                       # install from GitHub (main branch)
#   ./install.sh --source ~/src/codegraph233
#   ./install.sh --scope both          # project-level AND user-level MCP configs
#   ./install.sh --no-mcp
#   ./install.sh --ref v0.1.0
set -eu

MODULE="github.com/neko233-com/codegraph233"
SOURCE=""
REF="main"
DO_MCP=1
CLIENTS="codex,cursor,vscode,air,cowork,workbuddy,project"
SCOPE="project"

while [ $# -gt 0 ]; do
  case "$1" in
    --source) SOURCE="$2"; shift 2 ;;
    --ref) REF="$2"; shift 2 ;;
    --no-mcp) DO_MCP=0; shift ;;
    --clients) CLIENTS="$2"; shift 2 ;;
    --scope) SCOPE="$2"; shift 2 ;;
    -h|--help)
      sed -n '2,10p' "$0"; exit 0 ;;
    *) echo "unknown option: $1" >&2; exit 2 ;;
  esac
done

if ! command -v go >/dev/null 2>&1; then
  echo "ERROR: Go not found. Install Go 1.24+ (1.27 recommended): https://go.dev/dl/" >&2
  exit 1
fi
echo "Go detected: $(go version)"

if [ -n "$SOURCE" ]; then
  [ -f "$SOURCE/go.mod" ] || { echo "ERROR: $SOURCE has no go.mod" >&2; exit 1; }
  echo "Installing from local source: $SOURCE"
  ( cd "$SOURCE" && go install "./cmd/codegraph233" )
else
  echo "Installing $MODULE/cmd/codegraph233@$REF ..."
  go install "$MODULE/cmd/codegraph233@$REF"
fi

GOBIN_DIR="$(go env GOBIN)"
if [ -z "$GOBIN_DIR" ]; then
  GOBIN_DIR="$(go env GOPATH)/bin"
fi
BIN="$GOBIN_DIR/codegraph233"
[ -x "$BIN" ] || { echo "ERROR: expected binary at $BIN" >&2; exit 1; }

case ":$PATH:" in
  *":$GOBIN_DIR:"*) ;;
  *)
    echo "NOTE: add $GOBIN_DIR to PATH (e.g. echo 'export PATH=\"$GOBIN_DIR:\$PATH\"' >> ~/.profile)"
    ;;
esac
export PATH="$GOBIN_DIR:$PATH"

"$BIN" version

if [ "$DO_MCP" -eq 1 ] && [ -f ".codegraph233.json" ]; then
  echo "Registering MCP server for: $CLIENTS"
  "$BIN" install-mcp --clients "$CLIENTS" --exe "codegraph233" --scope "$SCOPE" || true
fi

echo
echo "Done. Next steps:"
echo "  codegraph233 init      # write .codegraph233.json (auto-detects engine + roots)"
echo "  codegraph233 index     # build/update the index"
echo "  codegraph233 status    # show detection and counts"
