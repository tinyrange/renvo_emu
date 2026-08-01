#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
limit=${RENVO_RUST_FILE_LINE_LIMIT:-1500}
failures=0

find "$repo_root/crates" -path '*/src/*.rs' -type f -print |
    LC_ALL=C sort |
    while IFS= read -r source
    do
        lines=$(wc -l < "$source")
        if [ "$lines" -gt "$limit" ]
        then
            printf '%s: %s lines (limit %s)\n' "${source#"$repo_root/"}" "$lines" "$limit" >&2
            failures=1
        fi
    done

# The pipeline above runs in a subshell on POSIX shells, so repeat the predicate
# without output to provide a reliable exit status.
if find "$repo_root/crates" -path '*/src/*.rs' -type f -exec sh -c '
    limit=$1
    shift
    for source do
        [ "$(wc -l < "$source")" -le "$limit" ] || exit 1
    done
' sh "$limit" {} +
then
    printf 'Rust source layout passed: every crate source is at most %s lines\n' "$limit"
else
    failures=1
fi

exit "$failures"
