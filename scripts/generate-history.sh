#!/usr/bin/env bash
#
# Writes a generated repository for the measurements of Q-2 and Q-3: a bare repository written
# through fast-import at fixed times, so the same size and shape give the same history each time.
#
# Shapes:
#   line  a main line with a feature branch of five commits merged back every fifty (default).
#   wide  thirty branches committed to in turn beside the main line, one merged back into it every
#         fifty commits and going on from the merge, so branch lines run long and side by side;
#         every ninety-seventh commit is dated before the one before it, as a wrong clock does.
#
# Usage:
#     scripts/generate-history.sh DIR COMMITS [SHAPE]
# Exit code 0 when the repository is written, 1 otherwise.

set -uo pipefail

if (($# < 2 || $# > 3)); then
  echo "Usage: scripts/generate-history.sh DIR COMMITS [line|wide]" >&2
  exit 1
fi
shape="${3:-line}"

git init --quiet --bare "$1" || exit 1
case "$shape" in
  line)
    awk -v n="$2" 'BEGIN {
      t = 1700000000; mark = 0; main = 0
      for (i = 1; i <= n; i++) {
        if (i % 50 == 0 && main > 0 && i + 6 <= n) {
          base = main
          for (j = 1; j <= 5; j++) {
            printf "commit refs/heads/feature\nmark :%d\ncommitter F <f@example.invalid> %d +0000\ndata 7\nfeature\nfrom :%d\n\n", ++mark, ++t, (j == 1 ? base : mark - 1)
            i++
          }
          printf "commit refs/heads/main\nmark :%d\ncommitter F <f@example.invalid> %d +0000\ndata 5\nmerge\nfrom :%d\nmerge :%d\n\n", ++mark, ++t, main, mark - 1
          main = mark
          continue
        }
        printf "commit refs/heads/main\nmark :%d\ncommitter F <f@example.invalid> %d +0000\ndata 4\nmain\n", ++mark, ++t
        if (main > 0) printf "from :%d\n", main
        printf "\n"
        main = mark
      }
    }' | git -C "$1" fast-import --quiet || exit 1
    ;;
  wide)
    awk -v n="$2" 'BEGIN {
      t = 1700000000; k = 30
      printf "commit refs/heads/main\nmark :1\ncommitter F <f@example.invalid> %d +0000\ndata 4\nroot\n\n", ++t
      main = 1
      for (b = 1; b < k; b++) tip[b] = 1
      for (mark = 2; mark <= n; mark++) {
        if (mark % 97 == 0) when = t - 3600; else when = ++t
        b = mark % k
        if (b == 0 && mark % 50 == 0) {
          m = 1 + int(mark / 50) % (k - 1)
          printf "commit refs/heads/main\nmark :%d\ncommitter F <f@example.invalid> %d +0000\ndata 5\nmerge\nfrom :%d\nmerge :%d\n\n", mark, when, main, tip[m]
          main = mark
          tip[m] = mark
        } else if (b == 0) {
          printf "commit refs/heads/main\nmark :%d\ncommitter F <f@example.invalid> %d +0000\ndata 4\nmain\nfrom :%d\n\n", mark, when, main
          main = mark
        } else {
          printf "commit refs/heads/b%d\nmark :%d\ncommitter F <f@example.invalid> %d +0000\ndata 4\nwork\nfrom :%d\n\n", b, mark, when, tip[b]
          tip[b] = mark
        }
      }
    }' | git -C "$1" fast-import --quiet || exit 1
    ;;
  *)
    echo "generate-history: no shape '$shape' (line or wide)" >&2
    exit 1
    ;;
esac
git -C "$1" symbolic-ref HEAD refs/heads/main
