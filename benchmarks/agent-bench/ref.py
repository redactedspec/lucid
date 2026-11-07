# Reference answers for tasks.md. Run from this directory to rewrite expected/.
import collections

out = {}


def emit(n, *lines):
    out[n] = "\n".join(str(x) for x in lines) + "\n"


c = collections.Counter("the quick brown fox jumps over the lazy dog the fox".split(" "))
emit(1, *[f"{w}: {n}" for w, n in sorted(c.items(), key=lambda p: (-p[1], p[0]))])


def caesar(s, k):
    r = ""
    for ch in s:
        if "a" <= ch <= "z":
            r += chr((ord(ch) - 97 + k) % 26 + 97)
        elif "A" <= ch <= "Z":
            r += chr((ord(ch) - 65 + k) % 26 + 65)
        else:
            r += ch
    return r


def pal(s):
    t = [x.lower() for x in s if x.isascii() and x.isalpha()]
    return str(t == t[::-1]).lower()


e = caesar("Attack at Dawn!", 3)
emit(2, e, caesar(e, -3), pal("A man, a plan, a canal: Panama"), pal("Hello"))


def rpn(src):
    st = []
    for t in src.split(" "):
        if t.isdigit():
            st.append(int(t))
        elif t in ("+", "-", "*", "/"):
            if len(st) < 2:
                return "underflow"
            b, a = st.pop(), st.pop()
            st.append({"+": a + b, "-": a - b, "*": a * b, "/": a // b if b else 0}[t])
        else:
            return f"bad token {t}"
    return f"leftover {len(st)}" if len(st) != 1 else st[0]


emit(3, *[f"{s} => {rpn(s)}" for s in ["3 4 + 2 *", "100 7 / 2 -", "1 +", "1 2", "1 x +"]])

inv = [["apple", 0.5, 10], ["pear", 0.75, 4], ["kiwi", 2.0, 1]]


def sell(name, n):
    for it in inv:
        if it[0] == name:
            if it[2] < n:
                return "false"
            it[2] -= n
            return "true"
    return "false"


log = [sell("apple", 3), sell("pear", 5), sell("kiwi", 1), sell("plum", 1)]
inv.append(["plum", 1.0, 2])
for it in inv:
    it[1] *= 1.0 - 10.0 / 100.0
inv = [i for i in inv if i[2] != 0]
total = 0.0
for it in inv:
    total += it[1] * it[2]
emit(4, *log, *[f"{a} {b} {q}" for a, b, q in inv], total)

sh = [("circle", 12.0), ("rect", 6.0), ("triangle", 10.0), ("rect", 1.0), ("circle", 3.0)]
cnt = collections.Counter(n for n, _ in sh)
emit(
    5,
    *[f"{n} {a}" for n, a in sorted(sh, key=lambda p: -p[1])],
    f"total area {sum(a for _, a in sh)}",
    *[f"{n}: {k}" for n, k in cnt.items()],
)

emit(6, [20, 30, 40, 50, 60, 70, 80], 3, "true", "false", 20, "none")

emit(7, 1, 2, 3, 4, "none", 2, "b", "b", "a", "none", [1, 4, 9], [2, 4], 6, [1, 2])


def calc(v):
    return ((v + 3) * 2 + 1) * 10


emit(8, calc(1), 42, 5, calc(0), 7)

emit(9, "$3.05", "$3.00", "none", "$0.05", "$0.20", "$3.00", 2, "point 1,2", "money $1.05")

b = "héllo wörld".encode()
h = 5381
for x in b:
    h = (h * 33 + x) & 0xFFFFFFFF
p, q = 1, 0
for x in b:
    p = (p + x) % 65521
    q = (q + p) % 65521
emit(10, f"{h:08x}", f"{q * 65536 + p:08x}", f"{sum(b) & 255:02x}")


def money(c):
    return f"${c // 100}.{c % 100:02}"


# 11
def parse_int(s):
    neg = s.startswith("-")
    body = s[1:] if neg else s
    if not any(ch.isdigit() for ch in body) and all(ch.isdigit() for ch in body):
        return "empty"
    for i, ch in enumerate(body):
        if not ("0" <= ch <= "9"):
            return f"bad char at {i + (1 if neg else 0)}"
    return int(s)


emit(11, *[f'"{s}" -> {parse_int(s)}' for s in ["42", "-17", "", "12a4", "-", "007", "4-2"]])

# 12
rows = "name,qty,cents\napple,3,25\npear,x,40\nkiwi,2,\nplum,5,10".split("\n")[1:]
lines, tot = [], 0
for r in rows:
    n, q, c = r.split(",")
    if q.isdigit() and c.isdigit():
        tot += int(q) * int(c)
        lines.append(f"{n}: {money(int(q) * int(c))}")
    else:
        lines.append(f"{n}: bad row")
emit(12, *lines, f"total: {money(tot)}")

# 13
RV = [(1000, "M"), (900, "CM"), (500, "D"), (400, "CD"), (100, "C"), (90, "XC"),
      (50, "L"), (40, "XL"), (10, "X"), (9, "IX"), (5, "V"), (4, "IV"), (1, "I")]


def to_roman(n):
    r = ""
    for v, s in RV:
        while n >= v:
            r += s
            n -= v
    return r


def from_roman(s):
    m = {"I": 1, "V": 5, "X": 10, "L": 50, "C": 100, "D": 500, "M": 1000}
    if any(c not in m for c in s):
        return "none"
    t = 0
    for i, c in enumerate(s):
        if i + 1 < len(s) and m[s[i + 1]] > m[c]:
            t -= m[c]
        else:
            t += m[c]
    return t


emit(13, *[to_roman(n) for n in [1, 4, 9, 14, 40, 90, 400, 1994, 2024, 3999]],
     *[from_roman(s) for s in ["MCMXCIV", "MMXXIV", "XLII", "IIII", "ABC"]])

# 14
import itertools
import re


def rle(s):
    return "".join(f"{len(list(g))}{k}" for k, g in itertools.groupby(s))


def unrle(s):
    return "".join(ch * int(n) for n, ch in re.findall(r"(\d+)(\D)", s))


enc = [rle(s) for s in ["aaabccdddd", "abc", "zzzzzzzzzzzz"]]
emit(14, *enc, *[unrle(e) for e in enc], unrle("12x3y"), f"[{rle('')}]")


# 15
def mul(a, b):
    return [[sum(a[i][k] * b[k][j] for k in range(len(b))) for j in range(len(b[0]))] for i in range(len(a))]


def tr(m):
    return [list(r) for r in zip(*m)]


A = [[1, 2, 3], [4, 5, 6]]
B = [[7, 8], [9, 10], [11, 12]]
row = lambda r: " ".join(map(str, r))
ab = mul(A, B)
emit(15, *map(row, ab), *map(row, tr(A)), *map(row, mul(tr(A), tr(B))), sum(ab[i][i] for i in range(len(ab))))


# 16
def sieve(n):
    s = [True] * n
    s[0] = s[1] = False
    for i in range(2, n):
        if s[i]:
            for j in range(i * i, n, i):
                s[j] = False
    return [i for i in range(n) if s[i]]


p100 = sieve(100)
emit(16, len(p100), row(p100), sieve(10000)[999])

# 17
bank = {}


def b_open(n, a):
    if a < 0:
        return f"bad amount {a}"
    bank[n] = a


def b_check(n, a):
    if n not in bank:
        return f"no account {n}"
    if a <= 0:
        return f"bad amount {a}"


def withdraw(n, a):
    e = b_check(n, a)
    if e:
        return e
    if a > bank[n]:
        return f"{n} is short by {a - bank[n]}"
    bank[n] -= a


def transfer(f, t, a):
    e = b_check(t, a) or withdraw(f, a)
    if e:
        return e
    bank[t] += a


res = [b_open("ada", 100), b_open("bob", 50), transfer("ada", "bob", 30), transfer("bob", "ada", 100),
       transfer("ada", "cy", 10), b_check("bob", -5), withdraw("ada", 70)]
emit(17, *["ok" if r is None else f"error: {r}" for r in res], *[f"{k}: {v}" for k, v in bank.items()])


# 18
def parse_config(src):
    d = {}
    for i, line in enumerate(src.split("\n"), 1):
        if line == "" or line.startswith("#"):
            continue
        if "=" not in line:
            return [f"line {i}: missing ="]
        k, v = line.split("=", 1)
        d[k.strip(" ")] = v.strip(" ")
    return [f"{k}={v}" for k, v in d.items()]


cfg = ["# settings", "name = demo", "port = 8080", "", "debug = true", "  port=9090", "timeout"]
emit(18, *parse_config("\n".join(cfg)), *parse_config("\n".join(cfg[:-1])))


# 19
class CalcErr(Exception):
    pass


def calc(s):
    pos = 0

    def skip():
        nonlocal pos
        while pos < len(s) and s[pos] == " ":
            pos += 1

    def factor():
        nonlocal pos
        skip()
        if pos >= len(s):
            raise CalcErr("unexpected end")
        if s[pos] == "(":
            pos += 1
            v = expr()
            skip()
            if pos >= len(s):
                raise CalcErr("unexpected end")
            if s[pos] != ")":
                raise CalcErr(f"unexpected character at {pos}")
            pos += 1
            return v
        if not s[pos].isdigit():
            raise CalcErr(f"unexpected character at {pos}")
        st = pos
        while pos < len(s) and s[pos].isdigit():
            pos += 1
        return int(s[st:pos])

    def term():
        nonlocal pos
        v = factor()
        while True:
            skip()
            if pos < len(s) and s[pos] in "*/":
                op = s[pos]
                pos += 1
                r = factor()
                if op == "/" and r == 0:
                    raise CalcErr("divide by zero")
                v = v * r if op == "*" else v // r
            else:
                return v

    def expr():
        nonlocal pos
        v = term()
        while True:
            skip()
            if pos < len(s) and s[pos] in "+-":
                op = s[pos]
                pos += 1
                r = term()
                v = v + r if op == "+" else v - r
            else:
                return v

    try:
        v = expr()
        skip()
        if pos < len(s):
            return f"unexpected character at {pos}"
        return v
    except CalcErr as e:
        return str(e)


emit(19, *[f"{s} => {calc(s)}" for s in
           ["1 + 2 * 3", "(1 + 2) * 3", "100 / 7 / 2", "2 * (3 + 4) - 5", "8 / (4 - 4)", "(1 + 2", "1 + * 2", "7 7"]])


# 20
def show(xs):
    return " -> ".join(map(str, xs)) if xs else "empty"


L = [1, 2, 3, 4, 5]
R = L[::-1]
emit(20, show(L), show(R), len(R), show([x for x in R if x % 2]), show([]), 0)

# 21
from collections import deque


def maze(rows):
    g = [list(r) for r in rows]
    h, w = len(g), len(g[0])
    st = next((r, c) for r in range(h) for c in range(w) if g[r][c] == "S")
    dist = {st: 0}
    q = deque([st])
    while q:
        r, c = q.popleft()
        for dr, dc in ((1, 0), (-1, 0), (0, 1), (0, -1)):
            nr, nc = r + dr, c + dc
            if 0 <= nr < h and 0 <= nc < w and g[nr][nc] != "#" and (nr, nc) not in dist:
                dist[(nr, nc)] = dist[(r, c)] + 1
                q.append((nr, nc))
    end = next((r, c) for r in range(h) for c in range(w) if g[r][c] == "E")
    return [dist.get(end, "no path"), len(dist)]


mz = ["S.#.....", ".##.###.", "....#...", ".##...#E", "...#...."]
mz2 = list(mz)
mz2[3] = mz2[3][:3] + "#" + mz2[3][4:]
mz3 = list(mz2)
mz3[2] = mz3[2][:2] + "#" + mz3[2][3:]
emit(21, *maze(mz), *maze(mz2), *maze(mz3))

# 22
import heapq

edges = [("A", "B", 4), ("A", "C", 2), ("B", "C", 1), ("B", "D", 5), ("C", "D", 8), ("C", "E", 10),
         ("D", "E", 2), ("D", "F", 6), ("E", "F", 3)]
adj = {n: [] for n in "ABCDEFG"}
for a, b, wt in edges:
    adj[a].append((b, wt))
    adj[b].append((a, wt))
dist, prev = {"A": 0}, {}
pq = [(0, "A")]
while pq:
    d, u = heapq.heappop(pq)
    if d > dist[u]:
        continue
    for v, wt in adj[u]:
        if d + wt < dist.get(v, 1 << 60):
            dist[v], prev[v] = d + wt, u
            heapq.heappush(pq, (d + wt, v))
path = ["F"]
while path[-1] != "A":
    path.append(prev[path[-1]])
emit(22, *[f"{n}: {dist.get(n, 'unreachable')}" for n in sorted(adj)], " -> ".join(path[::-1]))


# 23
def order(needs):
    placed = []
    while True:
        ready = sorted(j for j in needs if j not in placed and all(n in placed for n in needs[j]))
        if not ready:
            break
        placed.append(ready[0])
    left = sorted(j for j in needs if j not in placed)
    return "cycle: " + " ".join(left) if left else " ".join(placed)


needs = {"cook": ["shop", "chop"], "chop": ["shop"], "eat": ["cook", "set_table"], "set_table": [],
         "shop": [], "wash": ["eat"]}
o1 = order(needs)
needs["shop"] = ["wash"]
emit(23, o1, order(needs))

# 24
par = list(range(10))


def find(x):
    while par[x] != x:
        x = par[x]
    return x


for a, b in [(0, 1), (2, 3), (1, 3), (5, 6), (7, 7), (8, 9), (9, 5)]:
    par[find(a)] = find(b)
groups = {}
for i in range(10):
    groups.setdefault(find(i), []).append(i)
gs = sorted(groups.values())
emit(24, len(gs), *map(row, gs))

# 25
iv = sorted([[1, 3], [8, 10], [2, 6], [15, 18], [17, 20], [6, 7]])
merged = []
for s, e in iv:
    if merged and s <= merged[-1][1]:
        merged[-1][1] = max(merged[-1][1], e)
    else:
        merged.append([s, e])
emit(25, *[f"{s}-{e}" for s, e in merged], sum(e - s for s, e in merged))

# 26
words = "Lucid is a small language for agents and people who want fast programs without pauses supercalifragilistic end".split(" ")
wl, cur = [], ""
for wd in words:
    if cur == "":
        cur = wd
    elif len(cur) + 1 + len(wd) <= 20:
        cur += " " + wd
    else:
        wl.append(cur)
        cur = wd
wl.append(cur)
emit(26, *[f"|{x:<20}|" for x in wl])

# 27
grp = {}
for wd in ["listen", "silent", "google", "enlist", "cat", "gogole", "act", "dog", "tac"]:
    grp.setdefault("".join(sorted(wd)), []).append(wd)
emit(27, *[" ".join(g) for g in grp.values()])


# 28
def lcs(a, b):
    t = [[0] * (len(b) + 1) for _ in range(len(a) + 1)]
    for i in range(1, len(a) + 1):
        for j in range(1, len(b) + 1):
            t[i][j] = t[i - 1][j - 1] + 1 if a[i - 1] == b[j - 1] else max(t[i - 1][j], t[i][j - 1])
    i, j, r = len(a), len(b), ""
    while i > 0 and j > 0:
        if a[i - 1] == b[j - 1]:
            r = a[i - 1] + r
            i -= 1
            j -= 1
        elif t[i - 1][j] >= t[i][j - 1]:
            i -= 1
        else:
            j -= 1
    return r


def edits(a, b):
    p = list(range(len(b) + 1))
    for i in range(1, len(a) + 1):
        c = [i] + [0] * len(b)
        for j in range(1, len(b) + 1):
            c[j] = min(p[j] + 1, c[j - 1] + 1, p[j - 1] + (a[i - 1] != b[j - 1]))
        p = c
    return p[-1]


emit(28, lcs("AGGTAB", "GXTXAYB"), lcs("ABCBDAB", "BDCABA"), edits("kitten", "sitting"), edits("flaw", "lawn"),
     edits("", "abc"))

# 29
items = [("map", 9, 150), ("compass", 13, 35), ("water", 153, 200), ("sandwich", 50, 160), ("glucose", 15, 60),
         ("tin", 68, 45), ("banana", 27, 60), ("apple", 39, 40), ("cheese", 23, 30)]
best = []
for mask in range(1 << len(items)):
    ch = [items[i] for i in range(len(items)) if mask >> i & 1]
    if sum(x[1] for x in ch) <= 200:
        best.append((sum(x[2] for x in ch), ch))
top = max(v for v, _ in best)
wins = [c for v, c in best if v == top]
assert len(wins) == 1, "knapsack optimum must be unique"
emit(29, top, sum(x[1] for x in wins[0]), *sorted(x[0] for x in wins[0]))


# 30
def queens(n):
    sols = []

    def go(cols):
        r = len(cols)
        if r == n:
            sols.append(list(cols))
            return
        for c in range(n):
            if all(c != pc and abs(c - pc) != r - pr for pr, pc in enumerate(cols)):
                go(cols + [c])

    go([])
    return sols


q6 = queens(6)[0]
emit(30, *[f"{n}: {len(queens(n))}" for n in range(1, 9)], *["".join("Q" if c == q else "." for c in range(6)) for q in q6])


# 31
def life(cells):
    nxt = set()
    for r in range(8):
        for c in range(8):
            k = sum((r + dr, c + dc) in cells for dr in (-1, 0, 1) for dc in (-1, 0, 1) if dr or dc)
            if k == 3 or (k == 2 and (r, c) in cells):
                nxt.add((r, c))
    return nxt


def grid(cells):
    return ["".join("#" if (r, c) in cells else "." for c in range(8)) for r in range(8)]


cells = {(0, 1), (1, 2), (2, 0), (2, 1), (2, 2)}
g0 = grid(cells)
for i in range(30):
    cells = life(cells)
    if i == 3:
        g4 = grid(cells)
emit(31, *g0, "--", *g4, f"population after 30: {len(cells)}")

# 32
data = [("apple", 3, 25), ("watermelon", 1, 399), ("fig", 12, 5)]
cellsd = [["Item", "Qty", "Price"]] + [[n, str(q), money(c)] for n, q, c in data]
wd = [max(len(r[i]) for r in cellsd) for i in range(3)]
fmt = lambda r: " | ".join([r[0].ljust(wd[0]), r[1].rjust(wd[1]), r[2].rjust(wd[2])])
emit(32, fmt(cellsd[0]), "-+-".join("-" * x for x in wd), *[fmt(r) for r in cellsd[1:]])

# 33
DIG = "0123456789abcdefghijklmnopqrstuvwxyz"


def to_base(n, b):
    if n == 0:
        return "0"
    s, m = "", abs(n)
    while m:
        s = DIG[m % b] + s
        m //= b
    return "-" + s if n < 0 else s


def from_base(s, b):
    try:
        return int(s, b)
    except ValueError:
        return "none"


emit(33, to_base(255, 2), to_base(255, 16), to_base(-255, 16), to_base(0, 2), to_base(123456789, 36),
     from_base("ff", 16), from_base("zz", 36), from_base("102", 2), from_base("-1010", 2))

# 34
import math

emit(34, math.factorial(30), 2 ** 100, sum(map(int, str(math.factorial(100)))))

# 35
shapes = [("square 3", 9), ("rect 2x5", 10), ("grid 2x2 of 2", 16), ("square 1", 1), ("rect 3x3", 9)]
emit(35, *[f"{l} {a}" for l, a in sorted(shapes, key=lambda p: (-p[1], p[0]))], f"total {sum(a for _, a in shapes)}")

# 36
import bisect

ns = sorted([5, 3, 9, 1, 7, 3])
ts = sorted(["pear", "apple", "fig", "date"])


def bsearch(xs, t):
    i = bisect.bisect_left(xs, t)
    return i if i < len(xs) and xs[i] == t else "none"


emit(36, row(ns), bsearch(ns, 7), bsearch(ns, 4), bisect.bisect_left(ns, 4), bisect.bisect_left(ns, 10),
     row(ts), bsearch(ts, "fig"))

# 37
hp = []
o37 = []
for x in [5, 1, 8, 3, 9, 2]:
    heapq.heappush(hp, x)
o37 += [heapq.heappop(hp) for _ in range(3)]
heapq.heappush(hp, 0)
o37.append(len(hp))
while hp:
    o37.append(heapq.heappop(hp))
o37.append("none")
jobs = sorted([(2, "write"), (1, "plan"), (3, "test"), (1, "design")])
emit(37, *o37, *[f"{p} {n}" for p, n in jobs])

# 38
from collections import OrderedDict

cache, o38 = OrderedDict(), []


def put(k, v):
    cache[k] = v
    cache.move_to_end(k)
    if len(cache) > 3:
        o38.append(f"evict {cache.popitem(last=False)[0]}")


def get(k):
    if k not in cache:
        o38.append("none")
    else:
        cache.move_to_end(k)
        o38.append(cache[k])


put("a", 1); put("b", 2); put("c", 3); get("a"); put("d", 4); get("b"); get("c"); put("e", 5)
put("c", 30); get("a"); get("c"); get("d"); get("e")
emit(38, *o38, " ".join(cache))

# 39
menu = {"A1": ("cola", 75), "A2": ("water", 50)}
credit, o39 = 0, []
for ev in ["coin 25", "coin 25", "select A1", "coin 50", "select A1", "select B2", "refund", "coin 10", "refund",
           "select A2"]:
    p = ev.split(" ")
    if p[0] == "coin":
        credit += int(p[1])
        o39.append(f"credit {credit}")
    elif p[0] == "select":
        if p[1] not in menu:
            o39.append(f"unknown {p[1]}")
        elif credit < menu[p[1]][1]:
            o39.append(f"need {menu[p[1]][1] - credit} more")
        else:
            ch = credit - menu[p[1]][1]
            o39.append(f"dispense {menu[p[1]][0]}" + (f", change {ch}" if ch else ""))
            credit = 0
    else:
        o39.append(f"refund {credit}" if credit else "nothing to refund")
        credit = 0
emit(39, *o39)

# 40
import json

doc = json.loads('{"store": "north", "orders": [{"id": 1, "items": [{"sku": "a", "qty": 2, "cents": 150}], "note": "rush"}, {"id": 2, "items": [{"sku": "b", "qty": 1, "cents": 999}, {"sku": "a", "qty": 3, "cents": 150}]}]}')
o40 = [f"store {doc['store']}"]
sku = {}
for od in doc["orders"]:
    t = sum(i["qty"] * i["cents"] for i in od["items"])
    o40.append(f"order {od['id']}: {money(t)}" + (f" {od['note']}" if od.get("note") else ""))
    for i in od["items"]:
        sku[i["sku"]] = sku.get(i["sku"], 0) + i["qty"]
emit(40, *o40, *[f"{k}: {sku[k]}" for k in sorted(sku)], "bad input")

# 41
emit(41, "a.txt", "c.txt", "one+more", "false", "not found", 1)

# 42
argv = "total 4 x 15 -3 7".split(" ")
o42, t42 = [f"args {len(argv)}"], 0
for a in argv[1:]:
    if re.fullmatch(r"-?\d+", a):
        t42 += int(a)
    else:
        o42.append(f"skip {a}")
emit(42, *o42, f"total {t42}")

# 43
emit(43, "hello ada", "line 1", "line 2", "line 3", "missing: default")

# 44
emit(44, 50, "12.3 cm", "0.5 cm", "200.0 cm")


# 45
def stats(xs):
    s = sorted(xs)
    c = {x: xs.count(x) for x in xs}
    mode = min(x for x in c if c[x] == max(c.values()))
    return f"mean {sum(xs) // len(xs)} median {s[(len(s) - 1) // 2]} mode {mode}"


emit(45, stats([7, 1, 3, 3, 9, 4, 3, 8, 1]), stats([2, 2, 5, 5, 1]))

# 46
import calendar
import datetime

ds = ["2000-01-01", "2026-11-07", "1969-07-20", "2024-02-29"]
emit(46, *[f"{d} {calendar.day_name[datetime.date.fromisoformat(d).weekday()]}" for d in ds],
     (datetime.date(2026, 11, 7) - datetime.date(2000, 1, 1)).days,
     *[f"{y} {str(calendar.isleap(y)).lower()}" for y in [1900, 2000, 2024, 2100]])

# 47
st, vals = 12345, []
for _ in range(15):
    st = (st * 1664525 + 1013904223) & 0xFFFFFFFF
    vals.append(st)
w = (200 * 200) & 0xFFFF
emit(47, *vals[:5], row([v % 6 + 1 for v in vals[5:]]), w - 0x10000 if w >= 0x8000 else w)

# 48
import collections

ws = re.findall(r"[A-Za-z]+", "The cat and the hat. The CAT sat; a hat, a mat, and THE end!")
cn = collections.Counter(x.lower() for x in ws)
longest = max(ws, key=len)
assert longest == next(x for x in ws if len(x) == len(longest))
emit(48, len(ws), len(cn), longest, *[f"{k} {v}" for k, v in sorted(cn.items(), key=lambda p: (-p[1], p[0]))[:3]],
     " ".join(x[:1].upper() + x[1:].lower() for x in "the QUICK brown fOX".split(" ")))

# 49
studs = [("ada", [90, 85, None, 92]), ("bob", [70, None, None, 65]), ("cy", [None, None]),
         ("dee", [88, 79, 95, 100])]
o49, bestn, besta = [], None, -1
for n, gr in studs:
    pr = [x for x in gr if x is not None]
    if not pr:
        o49.append(f"{n}: no grades")
        continue
    a = sum(pr) // len(pr)
    letter = "A" if a >= 90 else "B" if a >= 80 else "C" if a >= 70 else "F"
    o49.append(f"{n}: avg {a} {letter}, absent {len(gr) - len(pr)}")
    if a > besta:
        bestn, besta = n, a
emit(49, *o49, f"best {bestn}")

# 50
people = [("ana", 31, "oslo"), ("ben", 25, "rome"), ("cal", 31, "oslo"), ("dan", 40, "rome"), ("eve", 25, "oslo"),
          ("fay", 35, "lima")]
s1 = sorted(people, key=lambda p: (p[2], -p[1], p[0]))
s2 = sorted(s1, key=lambda p: p[1])
cities = collections.Counter(p[2] for p in people)
emit(50, *[f"{n} {a} {c}" for n, a, c in s1], " ".join(p[0] for p in s2), *[f"{c}: {cities[c]}" for c in sorted(cities)])

for n, s in out.items():
    open(f"expected/{n:02}.out", "w", encoding="utf8", newline="\n").write(s)
