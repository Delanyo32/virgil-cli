#!/usr/bin/env bash
# Bench harness for the DuckDB experiment branch.
#
# Usage:
#   ./bench_matrix.sh <virgil-binary> <engine-label> <flavor>
#
#   flavor = cozo | duckdb
#     cozo   — uses --cozoscript flag, .sqlite cache extension
#     duckdb — uses --sql flag, .duckdb cache extension
#
# Per repo: wipes the relevant cache file, cold-builds (counts symbols),
# then runs each of the 7 templates 3 times and takes the median wall
# time. RSS captured on the cold build via /usr/bin/time -l.
#
# Outputs CSV to stdout. Columns:
#   engine,repo,lang,files,phase,wall_s,user_s,sys_s,max_rss_mb
#
# Where `phase` is "parse" for the cold build, then
# "query_<template_name>" for each template run. Per-template values
# are the median of 3 runs (wall_s); user_s/sys_s/RSS are from the
# median run, so cpu_eff = user_s / wall_s is computed on matched
# observations.
#
# Each individual command is wrapped in a $PER_CMD_TIMEOUT-second hard
# kill (default 60s) so a hung Datalog rule doesn't stall the run.
# Timed-out runs are reported with wall_s=TIMEOUT.

set -u
BIN="${1:?usage: bench_matrix.sh <virgil-binary> <engine-label> <flavor>}"
ENGINE="${2:?missing engine label}"
FLAVOR="${3:?missing flavor (cozo|duckdb)}"
BIN="$(realpath "$BIN")"

case "$FLAVOR" in
  cozo)   QUERY_FLAG="--cozoscript"; COUNT_QUERY='?[count(s)] := *symbol{id: s}'; CACHE_GLOB='*.sqlite' ;;
  duckdb) QUERY_FLAG="--sql";        COUNT_QUERY='SELECT COUNT(*) FROM symbol';  CACHE_GLOB='*.duckdb' ;;
  *) echo "unknown flavor: $FLAVOR (want cozo|duckdb)" >&2; exit 2 ;;
esac

# (label, path, name, lang)
ROWS=(
  "ripgrep|/tmp/ripgrep|bench-ripgrep|rs"
  "gin|/tmp/gin|bench-gin|go"
  "openclaw/discord|/tmp/openclaw/extensions/discord|bench-discord|ts,tsx"
  "openclaw/ui|/tmp/openclaw/ui|bench-ui|ts,tsx"
  "tokio|/tmp/tokio|bench-tokio|rs"
  "django|/tmp/django|bench-django|py"
)

# Templates + a realistic param. Names match files in src/queries/builtin/.
TEMPLATES=(
  "find_function_by_name:name=main"
  "find_callers:name=parse"
  "find_callees:name=parse"
  "find_cycles:"
  "find_implementations_of:name=Display"
  "export_surface:"
  "import_depth:"
)

CACHE_DIR="$HOME/Library/Caches/virgil"
[[ -d "$HOME/.cache/virgil" ]] && CACHE_DIR="$HOME/.cache/virgil"

PER_CMD_TIMEOUT="${PER_CMD_TIMEOUT:-60}"

echo "engine,repo,lang,files,phase,wall_s,user_s,sys_s,max_rss_mb"

# Run a command under /usr/bin/time -l with a perl alarm timeout.
# Emits one pipe-separated record: wall|user|sys|rss_mb
# On timeout: emits TIMEOUT|NA|NA|NA.
run_timed() {
  out=$(/usr/bin/time -l perl -e 'alarm shift; exec @ARGV' "$PER_CMD_TIMEOUT" "$@" 2>&1 >/dev/null)
  rss_bytes=$(printf '%s\n' "$out" | awk '/maximum resident set size/ {print $1}')
  real_s=$(printf '%s\n' "$out" | awk '/^ *[0-9.]+ *real/ {print $1}' | head -1)
  user_s=$(printf '%s\n' "$out" | awk '/[0-9.]+ user/ {for(i=1;i<=NF;i++) if($i=="user") print $(i-1)}' | head -1)
  sys_s=$(printf '%s\n'  "$out" | awk '/[0-9.]+ sys/  {for(i=1;i<=NF;i++) if($i=="sys")  print $(i-1)}' | head -1)
  if [[ -z "$real_s" ]]; then
    echo "TIMEOUT|NA|NA|NA"
    return
  fi
  rss_mb="NA"
  [[ -n "$rss_bytes" ]] && rss_mb=$(( rss_bytes / 1024 / 1024 ))
  echo "${real_s}|${user_s:-NA}|${sys_s:-NA}|$rss_mb"
}

# Median-of-3 wrapper: runs the command 3 times, picks the run whose
# wall_s is the median, returns that run's full record. Falls back to
# the first non-TIMEOUT result if any 3 runs timed out.
median3_full() {
  local r1 r2 r3
  r1=$(run_timed "$@")
  r2=$(run_timed "$@")
  r3=$(run_timed "$@")
  # Sort by wall_s, picking middle. TIMEOUT sorts last (alpha).
  printf '%s\n%s\n%s\n' "$r1" "$r2" "$r3" | sort -t'|' -k1,1n | awk 'NR==2'
}

# Print one CSV line from a "wall|user|sys|rss" record.
emit_row() {
  local engine="$1" label="$2" lang="$3" files="$4" phase="$5" rec="$6"
  IFS='|' read -r wall user sys rss <<< "$rec"
  echo "$engine,$label,$lang,$files,$phase,$wall,$user,$sys,$rss"
}

for row in "${ROWS[@]}"; do
  IFS='|' read -r label path name lang <<< "$row"
  if [[ ! -d "$path" ]]; then
    echo "$ENGINE,$label,$lang,0,MISSING,NA,NA,NA,NA"
    continue
  fi
  files=$(find "$path" -type f | wc -l | tr -d ' ')

  "$BIN" --quiet projects create "$name" --path "$path" --lang "$lang" 2>/dev/null || true

  rm -rf $CACHE_DIR/$CACHE_GLOB 2>/dev/null
  parse_rec=$(run_timed "$BIN" --quiet projects query "$name" "$QUERY_FLAG" "$COUNT_QUERY" --rebuild)
  emit_row "$ENGINE" "$label" "$lang" "$files" "parse" "$parse_rec"

  for tspec in "${TEMPLATES[@]}"; do
    tname="${tspec%%:*}"
    tparam="${tspec#*:}"
    args=( --quiet projects query "$name" --template "$tname" )
    if [[ -n "$tparam" ]]; then
      args+=( --param "$tparam" )
    fi

    rec=$(median3_full "$BIN" "${args[@]}")
    emit_row "$ENGINE" "$label" "$lang" "$files" "query_$tname" "$rec"
  done
done
