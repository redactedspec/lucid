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

for n, s in out.items():
    open(f"expected/{n:02}.out", "w", encoding="utf8", newline="\n").write(s)
