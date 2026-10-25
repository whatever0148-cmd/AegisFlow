#!/usr/bin/env bash
# Interactive loader for AegisFlow credentials and endpoints (writes .env, mode 0600).
#
#   bash harness/configure_keys.sh
#
# Asks for every setting aegis-config knows (`aegis-daemon config keys`) with hidden input. Press
# Enter to keep the current value, type "-" to remove it. Each value goes to
# `aegis-daemon config set NAME` on stdin through the shell's printf builtin: it never appears in a
# command line, in the process list or in this terminal. Only set/unset is printed.
# AEGIS_DOTENV points the loader at another file.
#
# NEBIUS_BASE_URL (where NEBIUS_API_KEY is sent) can only be set here or in the environment, not
# from the console, and must be https:// (http:// only for 127.0.0.1, localhost or [::1]). A refused
# value is reported and the loop moves on to the next setting.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
if ! command -v cargo >/dev/null 2>&1 && [ -f "$HOME/.cargo/env" ]; then
  # shellcheck disable=SC1091
  . "$HOME/.cargo/env"
fi
cargo build --release --quiet -p aegis-daemon </dev/null
BIN="$ROOT/target/release/aegis-daemon"

if [ ! -t 0 ]; then
  echo "configure_keys.sh needs an interactive terminal (hidden prompts)." >&2
  exit 2
fi

echo "AegisFlow settings ($("$BIN" config status </dev/null | head -n 1 | sed 's/^settings file: //'))"
echo "Enter keeps the current value, '-' removes it. Input is hidden."
echo
while read -r name kind; do
  state="$("$BIN" config status </dev/null | grep "^${name}: " | sed "s/^${name}: //")"
  # Drop anything typed before this prompt, so a value can never land under the wrong name.
  while IFS= read -rs -t 0.05 _ </dev/tty; do :; done
  printf '%s [%s, currently %s]: ' "$name" "$kind" "$state"
  value=""
  IFS= read -rs value </dev/tty || value=""
  echo
  if [ -z "$value" ]; then
    continue
  elif [ "$value" = "-" ]; then
    "$BIN" config unset "$name" </dev/null >/dev/null || echo "  $name: not changed (see the message above)" >&2
  else
    # The CLI's error text names the setting and the rule, never the value.
    printf '%s' "$value" | "$BIN" config set "$name" >/dev/null || echo "  $name: not saved (see the message above)" >&2
  fi
  value=""
done < <("$BIN" config keys </dev/null)
unset value

echo
"$BIN" config status </dev/null | sed 1d
echo
echo "Environment variables override .env. Restart aegis-daemon (bash harness/console.sh) or save from"
echo "the console's Settings drawer to apply the new values to a running daemon."
