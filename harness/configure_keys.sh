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
# NEBIUS_BASE_URL (where NEBIUS_API_KEY is sent) is picked first from the allowlisted Nemotron
# providers that `aegis-daemon config providers` prints (Nebius Token Factory, NVIDIA NIM,
# OpenRouter), or "l" for a loopback test endpoint (http(s)://127.0.0.1, localhost or [::1]); any
# other URL is refused. NEBIUS_API_KEY is then the key for the selected provider. A refused value
# is reported and the loop moves on to the next setting.
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
echo

# Nemotron provider (NEBIUS_BASE_URL): a menu built from the allowlist in aegis-config.
ids=(); urls=(); labels=(); signups=(); tier1s=(); tier2s=(); current="-"
while IFS=$'\t' read -r mark id url label signup tier1 tier2; do
  ids+=("$id"); urls+=("$url"); labels+=("$label"); signups+=("$signup"); tier1s+=("$tier1"); tier2s+=("$tier2")
  if [ "$mark" = "*" ]; then current="$label"; fi
done < <("$BIN" config providers </dev/null)
if [ "$current" = "-" ]; then current="a loopback test endpoint or an unrecognised value"; fi
# Discards anything typed before the next prompt, so a pasted value can never land under the wrong
# name. macOS ships bash 3.2, whose `read -t` rejects fractional timeouts, so flush the terminal's
# input queue with tcflush(3) instead (python3 comes with the Command Line Tools).
flush_typeahead() {
  if command -v python3 >/dev/null 2>&1; then
    python3 -c 'import os, termios; fd = os.open("/dev/tty", os.O_RDONLY); termios.tcflush(fd, termios.TCIFLUSH); os.close(fd)' 2>/dev/null && return
  fi
  while IFS= read -rs -t 1 -n 10000 _ </dev/tty; do :; done
}

echo "Nemotron provider for recovery tiers 1-2 (NEBIUS_BASE_URL; currently $current):"
for i in "${!ids[@]}"; do
  printf '  %d) %s  %s\n' "$((i + 1))" "${labels[$i]}" "${urls[$i]}"
done
echo "  d) default (remove NEBIUS_BASE_URL: ${labels[0]})"
echo "  l) loopback test endpoint (http(s)://127.0.0.1, localhost or [::1])"
flush_typeahead
printf 'Choice [Enter keeps the current provider]: '
choice=""
IFS= read -r choice </dev/tty || choice=""
case "$choice" in
  "") ;;
  d|D)
    "$BIN" config unset NEBIUS_BASE_URL </dev/null >/dev/null || echo "  NEBIUS_BASE_URL: not changed (see the message above)" >&2 ;;
  l|L)
    printf 'Loopback base URL (shown as typed; not a secret): '
    url=""
    IFS= read -r url </dev/tty || url=""
    if [ -n "$url" ]; then
      printf '%s' "$url" | "$BIN" config set NEBIUS_BASE_URL >/dev/null || echo "  NEBIUS_BASE_URL: not saved (see the message above)" >&2
    fi ;;
  *)
    if [[ "$choice" =~ ^[0-9]+$ ]] && [ "$choice" -ge 1 ] && [ "$choice" -le "${#ids[@]}" ]; then
      n=$((choice - 1))
      if printf '%s' "${urls[$n]}" | "$BIN" config set NEBIUS_BASE_URL >/dev/null; then
        echo "  ${labels[$n]} selected. NEBIUS_API_KEY below is the key for it: ${signups[$n]}"
        missing=""
        if [ "${tier1s[$n]}" = "-" ]; then missing="NEBIUS_TIER1_MODEL"; fi
        if [ "${tier2s[$n]}" = "-" ]; then missing="${missing:+$missing and }NEBIUS_TIER2_MODEL"; fi
        if [ -n "$missing" ]; then
          echo "  ${labels[$n]} has no confirmed default model id for $missing: set it below"
          echo "  (pick a Nemotron id from GET ${urls[$n]}/models, or use “Test Nemotron provider” in the console, which lists them)."
        fi
      else
        echo "  NEBIUS_BASE_URL: not saved (see the message above)" >&2
      fi
    else
      echo "  Unknown choice; NEBIUS_BASE_URL not changed." >&2
    fi ;;
esac
echo

echo "Enter keeps the current value, '-' removes it. Input is hidden."
echo
while read -r name kind; do
  if [ "$name" = "NEBIUS_BASE_URL" ]; then continue; fi
  state="$("$BIN" config status </dev/null | grep "^${name}: " | sed "s/^${name}: //")"
  # Drop anything typed before this prompt, so a value can never land under the wrong name.
  flush_typeahead
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
