#!/usr/bin/env bash
# Side-by-side diff of two bench_matrix outputs.
#   ./bench_diff.sh baseline.txt slice-a.txt
set -u
a="$1"; b="$2"
join -a1 -a2 -e '?' -o '0,1.3,2.3,1.4,2.4' \
  <(grep -v '^repo\|^===' "$a" | awk '{print $1, $0}' | sort) \
  <(grep -v '^repo\|^===' "$b" | awk '{print $1, $0}' | sort) \
  | awk 'BEGIN{printf "%-22s %10s %10s %10s %10s %10s\n", "repo", "RSS_A", "RSS_B", "dRSS%", "T_A", "T_B"}
         {a=$2; b=$3; pct=(a+0?100*(b-a)/a:0); printf "%-22s %10s %10s %9.1f%% %10s %10s\n", $1, a, b, pct, $4, $5}'
