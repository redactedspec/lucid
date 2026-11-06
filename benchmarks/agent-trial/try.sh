#!/bin/sh
# Usage: sh try.sh NN file.lcd, from the trial directory. Runs one attempt at
# task NN, keeps a copy of it, and logs the result.
n=$1
file=$2
mkdir -p attempts
try=$(($(ls attempts | grep -c "^t${n}_a.*lcd$") + 1))
cp "$file" "attempts/t${n}_a${try}.lcd"
./lucid run "$file" > out.tmp 2> "attempts/t${n}_a${try}.err"
code=$?
if [ "$code" = 0 ] && cmp -s out.tmp "expected/$n.out"; then
    result=pass
else
    result=fail
fi
echo "task $n attempt $try exit $code $result" | tee -a attempts/log.txt
cat "attempts/t${n}_a${try}.err"
if [ "$code" = 0 ] && [ "$result" = fail ]; then
    diff out.tmp "expected/$n.out"
fi
rm -f out.tmp
