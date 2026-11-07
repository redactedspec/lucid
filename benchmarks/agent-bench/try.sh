#!/bin/sh
# Usage: sh try.sh ARM NN, from the workspace. ARM is lucid, python, or ts.
# Checks and runs task NN, keeps a copy of the attempt, and logs the result.
# The lucid arm needs ./lucid. The ts arm needs tsc, and TS_TYPES naming a
# directory that holds @types/node, or bun's global one is used.
arm=$1
n=$2
here=$(pwd)
case $arm in
lucid) ext=lcd ;;
python) ext=py ;;
ts) ext=ts ;;
*) echo "unknown arm $arm" >&2; exit 2 ;;
esac
case $n in
44) src=44; entry=44/main.$ext ;;
45) src=45; entry=45/app/main.$ext; [ $arm = lucid ] && entry=45/app ;;
*) src=$n.$ext; entry=$n.$ext ;;
esac
if [ ! -e "$src" ]; then
    echo "missing $src" >&2
    exit 2
fi
mkdir -p attempts
try=$(($(ls attempts | grep -c "^t${n}_a[0-9]*\(\.$ext\)\?$") + 1))
keep=attempts/t${n}_a$try
[ -d "$src" ] || keep=$keep.$ext
cp -r "$src" "$keep"
err=$here/attempts/t${n}_a$try.err

run=$(mktemp -d)
cp -r "$src" "$run/"
cd "$run" || exit 2

case $arm in
lucid) "$here/lucid" check "$entry" > "$err" 2>&1 ;;
python) find "$src" -name '*.py' -exec python -m py_compile {} + > "$err" 2>&1 ;;
ts)
    types=${TS_TYPES:-$(cygpath -m "$HOME/.bun/install/global/node_modules/@types" 2>/dev/null)}
    find "$src" -name '*.ts' -exec tsc --noEmit --strict --target es2022 --module nodenext \
        --allowImportingTsExtensions --typeRoots "$types" --types node {} + > "$err" 2>&1
    ;;
esac
if [ $? = 0 ]; then compiled=yes; else compiled=no; fi
rm -rf "$run"/*

code=-
result=fail
if [ $compiled = yes ]; then
    cp -r "$here/$src" "$run/"
    args=
    [ "$n" = 42 ] && args="total 4 x 15 -3 7"
    case $arm in
    lucid) cmd="$here/lucid run" ;;
    python) cmd=python ;;
    ts) cmd=node ;;
    esac
    # $args splits on spaces on purpose.
    unset BENCH_MISSING
    BENCH_USER=ada BENCH_LIMIT=3 PYTHONUTF8=1 $cmd "$entry" $args > "$run.raw" 2>> "$err"
    code=$?
    # Python on Windows writes CRLF, so compare without CR.
    tr -d '\r' < "$run.raw" > "$run.out"
    if [ $code = 0 ] && cmp -s "$run.out" "$here/expected/$n.out"; then
        result=pass
    fi
fi
cd "$here" || exit 2
echo "$arm task $n attempt $try compiled $compiled exit $code $result" | tee -a attempts/log.txt
cat "$err"
if [ "$code" = 0 ] && [ $result = fail ]; then
    diff "$run.out" "expected/$n.out"
fi
rm -rf "$run" "$run.raw" "$run.out"
