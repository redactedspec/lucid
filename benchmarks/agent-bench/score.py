# Usage: python score.py LOG... Sums up attempts/log.txt files from try.sh.
import collections
import sys

tries = collections.defaultdict(list)
for path in sys.argv[1:]:
    for line in open(path, encoding="utf8"):
        f = line.split()
        tries[f[0], int(f[2])].append((int(f[4]), f[6] == "yes", f[-1] == "pass"))

arms = collections.defaultdict(list)
for (arm, task), t in tries.items():
    t.sort()
    first = next((i for i, x in enumerate(t) if x[2]), None)
    arms[arm].append((task, t[0][1], t[0][2], first))

for arm, rows in sorted(arms.items()):
    passed = [r for r in rows if r[3] is not None]
    turns = sum(r[3] for r in passed)
    print(f"{arm}: {len(rows)} tasks")
    print(f"  compiled first try: {sum(r[1] for r in rows)}")
    print(f"  passed first try:   {sum(r[2] for r in rows)}")
    print(f"  passed in the end:  {len(passed)}")
    print(f"  repair turns:       {turns}, on {sum(r[3] > 0 for r in passed)} tasks")
    print(f"  never passed:       {' '.join(str(r[0]) for r in sorted(rows) if r[3] is None) or '-'}")
