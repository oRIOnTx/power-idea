# Shared setup for the scripts in this folder: finds the Rust toolchain and
# moves to the repository root. Source it: `source "$(dirname "$0")/env.sh"`.

# rustup from Homebrew is keg-only, so it is not on PATH by default.
for dir in /opt/homebrew/opt/rustup/bin /usr/local/opt/rustup/bin "$HOME/.cargo/bin"; do
  [[ -d "$dir" ]] && PATH="$dir:$PATH"
done
export PATH

if ! command -v cargo >/dev/null 2>&1; then
  echo "cargo not found: install rustup (brew install rustup) and run rustup-init" >&2
  exit 1
fi

cd "$(dirname "${BASH_SOURCE[0]}")/.."
