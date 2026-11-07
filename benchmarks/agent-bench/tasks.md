# Agent Benchmark Tasks

Fifty programming tasks. Each arm writes every task in one language: Lucid, Python, or TypeScript.

## Rules for every task

- A task is one program that takes no input, unless it says otherwise. Data is written in the program. Write the program to `NN.lcd`, `NN.py`, or `NN.ts`, where `NN` is the task number. Tasks 44 and 45 are directories.
- Output must match exactly, line by line. Every line ends with a line break.
- Use the task, type, field, and variant names given. In Python and TypeScript, a task is a function, a `type` is a class, and a `choice` is a set of classes or tagged objects.
- Use only the language and its standard library.
- Unless a task gives a form, print `true` and `false` in lower case, and `none` for a missing value. Print a float in the shortest form that reads back to the same value, always with a `.`: `3.0`, `0.675`, `7.6499999999999995`. Print a list of numbers as `[1, 2, 3]`.
- Each run starts in a new empty directory, with these environment variables set: `BENCH_USER=ada`, `BENCH_LIMIT=3`.
- Signatures use Lucid notation. `T?` is a `T` or none, `A or B` is either, `list[T]` and `map[K, V]` are collections, `text` is a string, and `u32` and `byte` are unsigned numbers of 32 and 8 bits. In Python and TypeScript, use the closest equivalent.

## 01 Word frequency

For the text `the quick brown fox jumps over the lazy dog the fox`, count each word. Words are split on single spaces. Print one line per word in the form `word: count`, by count from high to low, and alphabetically within a count.

## 02 Text tools

Write `caesar(s: text, shift: int) -> text`. It moves each ASCII letter forward by `shift` in the alphabet, wrapping around and keeping its case. Other characters stay. `shift` can be negative.

Write `is_palindrome(s: text) -> bool`. It ignores case, and looks only at ASCII letters.

Print, one per line: `caesar("Attack at Dawn!", 3)`, then that result passed through `caesar` with `-3`, then `is_palindrome("A man, a plan, a canal: Panama")`, then `is_palindrome("Hello")`.

## 03 RPN calculator

Write `eval_rpn(source: text) -> int or EvalError`, where `EvalError` is a `choice` with these variants: `Underflow`, `Leftover(count: int)`, `BadToken(token: text)`. Tokens are split on spaces. A token is a run of digits, or one of `+`, `-`, `*`, `/`, where `/` is floor division. The first problem found stops the evaluation. `Underflow` means an operator found too few values. `Leftover` means that the end was reached with more than one value on the stack, and holds how many.

For each of `3 4 + 2 *`, `100 7 / 2 -`, `1 +`, `1 2`, `1 x +`, print the input, then ` => `, then the result. A result prints as the number, or as `underflow`, `leftover 2`, or `bad token x`.

## 04 Inventory

Write a `type Item` with `name: text`, `price: float`, `qty: int`, and a `type Inventory` holding a list of items.

Write these, each changing an inventory:

- `restock(name: text, n: int)` adds to an item's `qty`. An unknown name adds an item with price `1.0`.
- `sell(name: text, n: int) -> bool` lowers `qty` and gives `true`. It gives `false` and changes nothing if the item is unknown or has too little.
- `discount(percent: float)` multiplies every price by `1.0 - percent / 100.0`.
- `drop_empty()` removes items with `qty` 0.

And `total(...) -> float`, the sum of `price * qty`.

Start with apple 0.5 qty 10, pear 0.75 qty 4, kiwi 2.0 qty 1. Then run, in order: sell apple 3, sell pear 5, sell kiwi 1, sell plum 1, restock plum 2, discount 10, drop_empty. Print each `sell` result as it happens, then each item as `name price qty` with spaces, then the total.

## 05 Shapes

Write a `choice Shape` with `Circle(radius: float)`, `Rect(width: float, height: float)`, and `Triangle(base: float, height: float)`. Use `3.0` for pi. Write `area`, and `name` that gives `circle`, `rect`, or `triangle`.

For Circle 2.0, Rect 2.0 by 3.0, Triangle 4.0 by 5.0, Rect 1.0 by 1.0, Circle 1.0:

1. Print each shape as `name area`, by area from high to low. No two areas are equal.
2. Print `total area`, a space, and the sum.
3. Print each name with its count as `name: count`, in order of first appearance in the list above.

## 06 Binary search tree

Write a `type Node` with `value: int`, `left: Node?`, `right: Node?`. Write these, working on a `Node?`: `insert` gives a tree with the value added, and a value already there changes nothing. `contains` gives a `bool`. `in_order` gives a sorted `list[int]`. `height` gives an `int`, and an empty tree has height 0. `smallest` gives an `int?`.

Insert 50, 30, 70, 20, 40, 60, 80, 30 in order. Print `in_order`, then `height`, then `contains` of 60, then `contains` of 65, then `smallest`, then `smallest` of an empty tree.

## 07 Generic containers

Write a generic `type Stack[T]` with tasks `push`, `pop` that gives `T?`, `peek` that gives `T?`, and `size`. Write a generic `type Queue[T]` built from two `Stack[T]` values, with `enqueue` and `dequeue`, where `dequeue` gives `T?`.

Write generic `map`, `filter`, and `reduce(items, start, f)`.

With a queue of `int`: enqueue 1, 2, 3, dequeue and print, enqueue 4, then dequeue and print until `none`, printing that `none` too. With a stack of `text`: push `a`, `b`, print `size`, `peek`, `pop`, `pop`, `pop`.

Print `map` of `[1, 2, 3]` with squares, `filter` of `[1, 2, 3, 4]` for even values, `reduce` of `[1, 2, 3]` from 0 with sum, and `map` of `["a", "bb"]` to lengths.

## 08 Event bus

An event bus maps an event name to a list of handlers. A handler takes an `int` and gives an `int`. Firing an event passes a value through its handlers in order, and gives the final value. An unknown event gives the value unchanged. The bus keeps a count of all handlers run so far, over all fires.

Write `compose(f, g)`, which gives a task that runs `f` and then `g`. Write `make_adder(n)` and `make_multiplier(n)`, which each give a task.

Register, for `double`, a multiplier of 2. For `calc`, an adder of 3, then a multiplier of 2, then `compose` of an adder of 1 with a multiplier of 10. Fire `calc` with 1, `double` with 21, `missing` with 5, and `calc` with 0, printing each result. Then print the count.

## 09 Traits

Write a `type Money` with `cents: int`, so that `print` shows `$1.05` for 105 cents, with two digits after the point. `+` and `<` work on it.

Write a `type Point` with `x: int`, `y: int`. Equal points are the same key in a `set`.

Write a trait `Describe`, with `describe(self) -> text`. `Money` and `Point` have it, and a `Point` describes as `point 1,2`, and a `Money` as `money $1.05`.

Write generic `largest(items)`, which gives the largest item of a list or `none` when it is empty, and `sort(items)` that puts a list in order.

Print `Money` 105 plus `Money` 200. Print `largest` of Money 5, 300, 20. Print `largest` of an empty list of `Money`. Print `sort` of Money 300, 5, 20 as a list, one item per line. Print the size of a set holding points (1,2), (1,2), (2,1). Then print `describe` for a mixed list of Point (1,2) and Money 105.

## 10 Checksums

Over the bytes of the UTF-8 text `héllo wörld`:

- `djb2`, a `u32`: start at 5381. For each byte, set `h` to `h * 33 + byte`, wrapping at 32 bits.
- `adler32`, a `u32`: start with `a = 1`, `b = 0`. For each byte, `a = (a + byte) % 65521` and `b = (b + a) % 65521`. The result is `b * 65536 + a`.
- `checksum8`, a `byte`: the sum of all bytes, wrapping at 8 bits.

Print each one on its own line as eight lowercase hex digits with leading zeros, except `checksum8` with two digits. Write `hex(value: u32, digits: int) -> text`.

## 11 Number parsing

Write `parse_int(s: text) -> int or ParseError`, where `ParseError` is a `choice` with `Empty` and `BadChar(at: int)`. A number is an optional `-` then decimal digits. The empty text, and `-` alone, are `Empty`. Otherwise the first character that does not fit gives `BadChar` with its index from 0.

For each of `42`, `-17`, the empty text, `12a4`, `-`, `007`, `4-2`, print the input in double quotes, then ` -> `, then the number, `empty`, or `bad char at N`.

## 12 CSV totals

The data is this text, with one line per row:

```text
name,qty,cents
apple,3,25
pear,x,40
kiwi,2,
plum,5,10
```

Fields are split on commas. Skip the header. For each row, print `name: ` then `qty * cents` as money, or `name: bad row` if `qty` or `cents` is not a whole number of digits. Then print `total: ` and the sum of the good rows as money. Money prints as `$` then whole dollars, `.`, and two digits of cents: `$0.75`.

## 13 Roman numerals

Write `to_roman(n: int) -> text` for 1 to 3999, with the subtractive forms `IV`, `IX`, `XL`, `XC`, `CD`, `CM`. Write `from_roman(s: text) -> int?`, which gives `none` if any character is not one of `IVXLCDM`. It adds each value, except that it subtracts a value followed by a larger one. It does not check that the form is canonical.

Print `to_roman` of 1, 4, 9, 14, 40, 90, 400, 1994, 2024, 3999, one per line. Then print `from_roman` of `MCMXCIV`, `MMXXIV`, `XLII`, `IIII`, `ABC`, one per line.

## 14 Run-length coding

Write `encode(s: text) -> text`, which writes each run of one character as its length and then the character: `aaab` is `3a1b`. Write `decode(s: text) -> text`, the reverse, where a count can have more than one digit. Inputs to `encode` hold no digits.

Print `encode` of `aaabccdddd`, `abc`, and `zzzzzzzzzzzz`, then `decode` of each of those three results, then `decode` of `12x3y`. Print the empty text's `encode` last, in brackets: `[]`.

## 15 Matrices

A matrix is a `list[list[int]]`, as a list of rows. Write `multiply(a, b)` and `transpose(m)`.

With `a = [[1, 2, 3], [4, 5, 6]]` and `b = [[7, 8], [9, 10], [11, 12]]`, print each row of `multiply(a, b)`, then each row of `transpose(a)`, then each row of `multiply(transpose(a), transpose(b))`. Print a row as its numbers with one space between them. Then print the sum of the diagonal of `multiply(a, b)`.

## 16 Primes

Write a sieve of Eratosthenes. Print how many primes are below 100. Print those primes on one line with one space between them. Then print the 1000th prime, counting 2 as the first.

## 17 Bank

Write a `choice BankError` with `NoAccount(name: text)`, `Short(name: text, by: int)`, and `BadAmount(amount: int)`. It is an error type, with the messages `no account NAME`, `NAME is short by N`, and `bad amount N`.

Write a `type Bank` that holds balances by name, and these tasks, each changing a bank and giving `none or BankError`:

- `open(name, amount)`. A negative amount is `BadAmount`.
- `deposit(name, amount)`. An unknown name is `NoAccount`. An amount that is not positive is `BadAmount`.
- `withdraw(name, amount)`. As `deposit`, and an amount larger than the balance is `Short`, by the difference.
- `transfer(from, to, amount)`, which uses `withdraw` and `deposit`. A failed transfer changes nothing.

Run these in order, printing `ok` or `error: ` and the message for each: open `ada` with 100, open `bob` with 50, transfer 30 from `ada` to `bob`, transfer 100 from `bob` to `ada`, transfer 10 from `ada` to `cy`, deposit -5 to `bob`, withdraw 70 from `ada`. Then print each balance as `name: amount`, in the order opened.

## 18 Config files

Write `parse_config(source: text) -> map[text, text] or ConfigError`. Each line is `key = value`. Remove spaces at both ends of the key and the value. Skip empty lines, and lines whose first character is `#`. A key seen again replaces the value, and keeps its first place. A line with no `=` is an error, with the message `line N: missing =`, where lines count from 1.

The first source is these 7 lines:

```text
# settings
name = demo
port = 8080

debug = true
  port=9090
timeout
```

The second source is the same without its last line. Parse both. For an error, print its message. For a result, print each entry as `key=value`, in order.

## 19 Calculator

Write `calc(source: text) -> int or CalcError`. `CalcError` is a `choice` with `DivideByZero`, `End`, and `Unexpected(at: int)`. The grammar, where spaces may appear between any two tokens:

```text
expr   = term (("+" | "-") term)*
term   = factor (("*" | "/") factor)*
factor = number | "(" expr ")"
```

A number is one or more decimal digits. `/` is floor division. Evaluate from left to right as you parse. `End` means the text ended where a factor or `)` was needed. `Unexpected` holds the index from 0 of the first character that does not fit, including one left over after a whole `expr`. The first problem found is the result.

For each of `1 + 2 * 3`, `(1 + 2) * 3`, `100 / 7 / 2`, `2 * (3 + 4) - 5`, `8 / (4 - 4)`, `(1 + 2`, `1 + * 2`, `7 7`, print the input, ` => `, then the number, `divide by zero`, `unexpected end`, or `unexpected character at N`.

## 20 Linked list

Write a `type Node` with `value: int` and `next: Node?`. Write `from_list(items: list[int]) -> Node?`, `show(list: Node?) -> text`, `reverse(list: Node?) -> Node?`, `length(list: Node?) -> int`, and `remove_even(list: Node?) -> Node?`. `show` joins the values with ` -> `, and gives `empty` for an empty list.

Build a list from 1 to 5. Print `show` of it, of its reverse, its length, and `show` of `remove_even` of the reverse. Then print `show` and `length` of the reverse of an empty list.

## 21 Maze

A maze is text, one row per line. `#` is a wall, `.` is open, `S` is the start, and `E` is the end. A step moves up, down, left, or right onto a cell that is not a wall, and never leaves the maze.

```text
S.#.....
.##.###.
....#...
.##...#E
...#....
```

Print the fewest steps from `S` to `E`, then how many cells, counting `S`, can be reached from `S`. Then change the cell at row 3, column 3, counting from 0, to `#`, and print both again. Then also change row 2, column 2 to `#`, and print both again. Print `no path` instead of the steps if `E` cannot be reached.

## 22 Shortest paths

An undirected graph has these edges with weights: A-B 4, A-C 2, B-C 1, B-D 5, C-D 8, C-E 10, D-E 2, D-F 6, E-F 3. It also has a node G with no edges.

Find the shortest distance from A to every node. Print each node in alphabetical order as `X: distance`, or `X: unreachable`. Then print the shortest path from A to F, as its nodes joined with ` -> `.

## 23 Build order

Each job lists the jobs it needs first:

- `cook` needs `shop` and `chop`
- `chop` needs `shop`
- `eat` needs `cook` and `set_table`
- `set_table` needs nothing
- `shop` needs nothing
- `wash` needs `eat`

Write `order(needs: map[text, list[text]]) -> list[text] or Cycle`, where `Cycle` holds the jobs that could not be placed. Repeatedly take the job that comes first alphabetically among those whose needs are all placed. Print the order with one space between jobs.

Then add that `shop` needs `wash`, and run it again. Print `cycle: ` and the jobs that could not be placed, alphabetically, with one space between them.

## 24 Groups

There are 10 items, numbered 0 to 9. Join these pairs into groups: 0 and 1, 2 and 3, 1 and 3, 5 and 6, 7 and 7, 8 and 9, 9 and 5. Use union-find.

Print the number of groups. Then print each group as its items in increasing order with one space between them, ordered by each group's smallest item.

## 25 Intervals

Merge the intervals `[1, 3]`, `[8, 10]`, `[2, 6]`, `[15, 18]`, `[17, 20]`, `[6, 7]`. Two intervals merge when one starts at or before the end of the other. Print each merged interval as `start-end`, in order. Then print the total length covered, the sum of `end - start`.

## 26 Word wrap

Wrap this text to lines of at most 20 characters:

```text
Lucid is a small language for agents and people who want fast programs without pauses supercalifragilistic end
```

Words are split on single spaces. Fill each line with as many words as fit, with one space between words. A word longer than 20 characters gets a line of its own. Print each line between `|` characters, padded with spaces on the right to 20 characters: `|Lucid is a small    |`.

## 27 Anagrams

Group these words by their letters: `listen`, `silent`, `google`, `enlist`, `cat`, `gogole`, `act`, `dog`, `tac`. Print each group on one line, with its words in input order and one space between them. Order the groups by the first appearance of any of their words.

## 28 Sequences

Write `lcs(a: text, b: text) -> text`. Fill a table where `t[i][j]` is the length of the longest common subsequence of the first `i` characters of `a` and the first `j` of `b`. Then walk back from `i = len(a)`, `j = len(b)`: if the characters before `i` and `j` are equal, take that character and step both back. Otherwise step `i` back if `t[i - 1][j] >= t[i][j - 1]`, and step `j` back if not.

Write `edits(a: text, b: text) -> int`, the Levenshtein distance.

Print `lcs` of `AGGTAB` and `GXTXAYB`, then of `ABCBDAB` and `BDCABA`. Then print `edits` of `kitten` and `sitting`, of `flaw` and `lawn`, and of the empty text and `abc`.

## 29 Knapsack

Items, as name, weight, value: `map` 9 150, `compass` 13 35, `water` 153 200, `sandwich` 50 160, `glucose` 15 60, `tin` 68 45, `banana` 27 60, `apple` 39 40, `cheese` 23 30. Choose items with a total weight of at most 200 and the largest total value. Each item is chosen at most once.

Print the total value, then the total weight, then the names of the chosen items in alphabetical order, one per line.

## 30 Queens

Count the ways to place `n` queens on an `n` by `n` board so that no two attack each other. Print `n: count` for `n` from 1 to 8.

Then print the first solution for `n = 6`, where solutions are ordered by the column of the queen in row 0, then in row 1, and so on. Print one row per line, as `Q` for the queen and `.` for every other cell.

## 31 Game of life

An 8 by 8 grid starts with live cells at these row, column pairs, counting from 0: (0, 1), (1, 2), (2, 0), (2, 1), (2, 2). Cells outside the grid are always dead. In each generation, a live cell with 2 or 3 live neighbors stays alive, a dead cell with exactly 3 becomes alive, and every other cell is dead. Neighbors are the 8 cells around a cell.

Print the grid, then a line `--`, then the grid after 4 generations. Print a grid one row per line, as `#` for live and `.` for dead. Then print `population after 30: ` and the number of live cells after 30 generations.

## 32 Table

Print this data as a table: `apple` 3 at 25 cents, `watermelon` 1 at 399 cents, `fig` 12 at 5 cents. The columns are `Item`, `Qty`, and `Price`, with price as money, `$3.99`. Each column is as wide as its widest cell, header included. `Item` is aligned left, and the others right. Columns are joined with ` | `.

The first line is the header. The second is `-` for the width of each column, joined with `-+-`. Then one line per item, in the order given.

## 33 Number bases

Write `to_base(n: int, base: int) -> text` for bases 2 to 36, with digits `0` to `9` then `a` to `z`, and a leading `-` for a negative number. Write `from_base(s: text, base: int) -> int?`, which reads the same form, and gives `none` for a digit that is not valid in the base.

Print `to_base` of 255 in base 2, 255 in base 16, -255 in base 16, 0 in base 2, and 123456789 in base 36. Then print `from_base` of `ff` in base 16, `zz` in base 36, `102` in base 2, and `-1010` in base 2.

## 34 Big numbers

Print 30 factorial, then 2 to the power 100, then the sum of the decimal digits of 100 factorial. Each value is exact.

## 35 Shapes as a trait

Write a trait `Shape` with `area(self) -> int` and `label(self) -> text`. Write three types that have it:

- `Square` with `side`. Its label is `square 3`.
- `Rect` with `width` and `height`. Its label is `rect 2x5`.
- `Grid` with `cols`, `rows`, and `cell`, an area of `cols * rows * cell * cell`. Its label is `grid 2x2 of 2`.

Put these in one list: Square 3, Rect 2 by 5, Grid 2 by 2 of 2, Square 1, Rect 3 by 3. Sort it by area from high to low, and by label alphabetically within an area. Print each as its label, a space, and its area. Then print `total ` and the sum of the areas.

## 36 Generic search

Write generic `sort(items)`, using merge sort, and `binary_search(items, target) -> int?`, which gives an index of `target` in a sorted list, or `none`. Write `lower_bound(items, target) -> int`, the first index whose item is not less than `target`. All three work on any ordered type.

Sort `[5, 3, 9, 1, 7, 3]` and print it with one space between items. Print `binary_search` of 7 and of 4, and `lower_bound` of 4 and of 10. Then sort `["pear", "apple", "fig", "date"]`, print it the same way, and print `binary_search` of `fig`.

## 37 Heap

Write a generic `type Heap[T]`, a binary min-heap stored in a list, with `push`, `pop` that gives `T?`, and `size`.

With a heap of `int`: push 5, 1, 8, 3, 9, 2, then pop three times and print each. Push 0. Print the size. Then pop and print until `pop` gives `none`, and print that `none` too, all one per line.

Write a `type Job` with `priority: int` and `name: text`, ordered by priority and then by name. Push jobs 2 `write`, 1 `plan`, 3 `test`, 1 `design` into a `Heap[Job]`. Pop all of them, and print each as `priority name`.

## 38 Cache

Write an LRU cache from `text` to `int` with a capacity of 3. `get(key)` gives `int?`, and makes the key the most recent. `put(key, value)` adds or replaces, and makes the key the most recent. When a `put` makes the cache hold more than 3 keys, it removes the least recent one, and prints `evict ` and that key.

Run in order: put `a` 1, put `b` 2, put `c` 3, get `a`, put `d` 4, get `b`, get `c`, put `e` 5, put `c` 30, get `a`, get `c`, get `d`, get `e`. Print the result of each `get`. Then print the keys from least to most recent, with one space between them.

## 39 Vending machine

A machine sells `A1` cola for 75 cents and `A2` water for 50. It holds a credit, which starts at 0. Handle these events in order, and print one line for each:

- `coin N` adds N to the credit, and prints `credit ` and the credit.
- `select CODE` prints `unknown CODE` for an unknown code. If the credit is too small, it prints `need N more`. Otherwise it prints `dispense NAME`, then `, change N` if any credit is left over, and the credit becomes 0.
- `refund` prints `refund N` and sets the credit to 0, or prints `nothing to refund` if it is 0.

The events: `coin 25`, `coin 25`, `select A1`, `coin 50`, `select A1`, `select B2`, `refund`, `coin 10`, `refund`, `select A2`. Store them as text, and read each event from its text.

## 40 JSON

This is the input, as text in the program:

```json
{"store": "north", "orders": [{"id": 1, "items": [{"sku": "a", "qty": 2, "cents": 150}], "note": "rush"}, {"id": 2, "items": [{"sku": "b", "qty": 1, "cents": 999}, {"sku": "a", "qty": 3, "cents": 150}]}]}
```

Read it with the standard JSON library into types: a store with a name and orders, an order with an id, items, and an optional note, and an item with `sku`, `qty`, and `cents`. Print `store north`. For each order, print `order ID: ` and the sum of `qty * cents` as money, `$3.00`, then a space and the note if there is one. Then print the total quantity of each sku, as `sku: qty`, by sku alphabetically.

Then read `{"store": 5, "orders": []}` the same way. It fails, since the store is not text: print `bad input`.

## 41 Files

Use the standard library to work with files, relative to the current directory:

1. Make a directory `data`.
2. Write `two` to `data/b.txt`, and then `one` to `data/a.txt`.
3. Append `+more` to `data/a.txt`.
4. Rename `data/b.txt` to `data/c.txt`.
5. Print the names in `data`, sorted, one per line.
6. Print the text of `data/a.txt`.
7. Print whether `data/b.txt` exists.
8. Read `data/missing.txt`. It fails because the file does not exist: print `not found`.
9. Remove `data/c.txt`, then print how many names are in `data`.

## 42 Arguments

The program is run with the arguments `total 4 x 15 -3 7`. Print `args ` and the number of arguments. If the first argument is `total`, go through the rest in order: for each one that is not a whole number with an optional `-`, print `skip ` and the argument. Then print `total ` and the sum of the numbers.

## 43 Environment

Read the environment variable `BENCH_USER`, and print `hello ` and its value. Read `BENCH_LIMIT`, a number, and print `line 1` up to `line N` for that number. Read `BENCH_MISSING`, which is not set, and print `missing: default`, using `default` as its value when it is not set.

## 44 Modules

Write a program in three files, in the directory `44`:

- `geometry`, which has a public type `Point` with `x: int` and `y: int`, and a public task `path_length2(points: list[Point]) -> int`, the sum of the squared distances between each point and the next. It uses a private task `dist2(a: Point, b: Point) -> int`.
- `units`, which has a public task `cm(mm: int) -> text` that shows a length in millimeters as centimeters with one decimal: `12.3 cm` for 123.
- `main`, which uses the other two. It prints `path_length2` of the points (0, 0), (3, 4), (6, 8), then `cm` of 123, 5, and 2000.

The files are `44/main.lcd`, `44/geometry.lcd`, and `44/units.lcd`, or the same with `.py` or `.ts`. The program runs as `44/main`.

## 45 Packages

Write a library `stats` and a program `app` that uses it, in the directories `45/stats` and `45/app`. In Lucid, each is a package, and `app` depends on `stats` by path. In Python and TypeScript, `45/app/main.py` or `45/app/main.ts` imports `45/stats` by relative path, and Python may change `sys.path` to do it.

`stats` has public tasks over a `list[int]`: `mean_floor`, the sum floor-divided by the count; `median`, the middle item of the sorted list, or the lower of the two middle items; and `mode`, the most frequent item, or the smallest of those that tie.

`app` prints, for `[7, 1, 3, 3, 9, 4, 3, 8, 1]` and then `[2, 2, 5, 5, 1]`, one line each: `mean M median D mode O`.

## 46 Dates

Write the date arithmetic yourself. Do not use a date or time library.

Print the weekday of 2000-01-01, 2026-11-07, 1969-07-20, and 2024-02-29, each as `YYYY-MM-DD Weekday`, with the weekday's English name. Then print the number of days from 2000-01-01 to 2026-11-07. Then print whether each of 1900, 2000, 2024, and 2100 is a leap year, as `YEAR true` or `YEAR false`.

## 47 Sized numbers

A random number generator keeps a 32-bit unsigned state, starting at 12345. Each step sets the state to `state * 1664525 + 1013904223`, wrapping at 32 bits, and gives the new state.

Print the first 5 values, one per line. Then print the next 10 values as dice rolls, `value % 6 + 1`, on one line with one space between them. Then print the product of 200 and 200 as a 16-bit signed number, wrapping.

## 48 Text statistics

A word is a run of ASCII letters. Words compare without case. For this text:

```text
The cat and the hat. The CAT sat; a hat, a mat, and THE end!
```

Print the number of words, then the number of different words. Print the longest word as written, the first one if more than one has that length. Then print the 3 most frequent words in lower case as `word count`, by count from high to low and alphabetically within a count.

Then print `the QUICK brown fOX` in title case: the first letter of each word in upper case, and the rest in lower case.

## 49 Grades

A `type Student` has `name: text` and `grades: list[int?]`, where `none` marks an absent test. The students are `ada` with 90, 85, none, 92; `bob` with 70, none, none, 65; `cy` with none, none; and `dee` with 88, 79, 95, 100.

For each student, print `name: avg A L, absent N`. `A` is the floor of the mean of the present grades. `L` is the letter grade: `A` from 90, `B` from 80, `C` from 70, and `F` below. `N` counts the absent tests. A student with no present grade prints `name: no grades`. Then print `best ` and the name with the highest average.

## 50 Sorting with a rule

Write `sort_by(items, less)`, a stable sort, where `less(a, b)` says whether `a` goes before `b`. A `type Person` has `name: text`, `age: int`, and `city: text`.

The people: `ana` 31 `oslo`, `ben` 25 `rome`, `cal` 31 `oslo`, `dan` 40 `rome`, `eve` 25 `oslo`, `fay` 35 `lima`.

Print the people sorted by city, then by age from high to low, then by name, each as `name age city`. Then sort that result by age alone, from low to high, and print the names on one line with one space between them. The sort is stable, so people of one age keep their order. Then print, for each city in alphabetical order, `city: count`.
