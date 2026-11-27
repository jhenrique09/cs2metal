#!/bin/sh
# Runs every fixture natively, inside the same SDK image that built it, and
# compares its standard output and exit status with expected/<name>.txt. With
# --update it writes the expected files instead. This is the ground truth the
# cs2metal loader must reproduce on macOS.
set -u

out=/out
expected=/src/expected
update=${1:-}
status=0

run() {
    name=$1
    shift
    actual=$(cd "$out" && env -i PATH=/usr/bin:/bin CS2METAL_FIXTURE=set "$@" 2>&1; echo "exit: $?")
    if [ "$update" = "--update" ]; then
        printf '%s\n' "$actual" > "$expected/$name.txt"
        echo "updated: $name"
    elif [ "$actual" = "$(cat "$expected/$name.txt" 2>/dev/null)" ]; then
        echo "ok: $name"
    else
        echo "FAILED: $name"
        printf '%s\n' "$actual" | diff "$expected/$name.txt" - || true
        status=1
    fi
}

run hello ./hello one "two words"
run tls ./tls
run stack_guard ./stack_guard
run cxx ./cxx
run dlopen ./dlopen
run ifunc ./ifunc
run scope ./scope
run tlsprobe ./tlsprobe
exit $status
