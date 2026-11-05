# Grammar

This document is normative. Notation is EBNF: `{ x }` repeats zero or more times, `[ x ]` is optional, and `"x"` is a literal token.

The grammar is context-free. Parsing never depends on declarations or types.

## Layout

The lexer produces `NEWLINE`, `INDENT`, and `DEDENT` tokens.

1. Blank lines and comment-only lines produce nothing.
2. Inside `()`, `[]`, or `{}`, newlines and indentation produce nothing.
3. Each other line ends with `NEWLINE`. At the start of the next line, compare indentation with the top of the indent stack. One level deeper is 4 more spaces and gives `INDENT`. Each level shallower gives `DEDENT`. Any other width is an error.
4. A tab character in leading whitespace is an error.
5. At end of file, the lexer emits `NEWLINE` if needed, then closes all open levels.

## Tokens

```ebnf
NAME     = (letter | "_") { letter | digit | "_" } ;   (* not a keyword *)
INT      = digit { digit | "_" } | "0x" hex { hex | "_" } | "0b" bin { bin | "_" } ;
FLOAT    = digit { digit | "_" } "." digit { digit | "_" } [ exponent ] ;
TEXT     = '"' { char | "{" expression "}" | "{{" | "}}" } '"'
         | '"""' multi-line form '"""' ;
```

A `FLOAT` needs digits on both sides of the point, so `0..10` lexes as a range.

## Declarations

```ebnf
file        = { item | statement } EOF ;

item        = use_decl | test_def
            | [ "public" ] ( task_def | type_def | choice_def | trait_def ) ;

use_decl    = "use" module_path [ "as" NAME ] NEWLINE ;
module_path = [ "./" ] NAME { "." NAME } ;

task_def    = task_sig NEWLINE block
            | "unsafe" "extern" task_sig NEWLINE ;
task_sig    = "task" NAME [ type_params ] "(" [ params ] ")" [ "->" type ] ;
type_params = "[" type_param { "," type_param } [ "," ] "]" ;
type_param  = NAME [ ":" type ] ;
params      = param { "," param } [ "," ] ;
param       = [ "change" | "take" ] ( "self" [ ":" type ] | NAME ":" type [ "=" expression ] ) ;

type_def    = "type" NAME [ type_params ]
              ( "=" type NEWLINE | NEWLINE INDENT { field } DEDENT ) ;
field       = NAME ":" type [ "=" expression ] NEWLINE ;

choice_def  = "choice" NAME [ type_params ] NEWLINE INDENT { variant } DEDENT ;
variant     = NAME [ "(" params ")" ] NEWLINE ;

trait_def   = "trait" NAME [ type_params ] NEWLINE INDENT { task_sig NEWLINE } DEDENT ;

test_def    = "test" TEXT NEWLINE block ;
```

## Types

```ebnf
type        = type_term { "or" type_term } ;
type_term   = type_prefix [ "?" ] ;
type_prefix = ( "shared" | "weak" | "any" ) type_prefix | type_atom ;
type_atom   = NAME { "." NAME } [ type_args ]
            | "none"
            | "task" "(" [ type { "," type } ] ")" [ "->" type ]
            | "(" type ")" ;
type_args   = "[" type { "," type } [ "," ] "]" ;
```

`weak Node?` means `(weak Node)?`.

## Statements

```ebnf
block       = INDENT { statement } DEDENT ;

statement   = let_stmt | var_stmt | if_stmt | while_stmt | for_stmt
            | match_expr | return_stmt | print_stmt | expect_stmt
            | unsafe_stmt | task_def | "break" NEWLINE | "continue" NEWLINE
            | "..." NEWLINE | expr_stmt ;

let_stmt    = "let" NAME [ ":" type ] "=" value ;
var_stmt    = "var" NAME [ ":" type ] "=" value ;
value       = expression NEWLINE | match_expr ;

if_stmt     = "if" expression NEWLINE block
              { "else" "if" expression NEWLINE block }
              [ "else" NEWLINE block ] ;
while_stmt  = "while" expression NEWLINE block ;
for_stmt    = "for" [ "change" ] NAME "in" expression NEWLINE block ;

match_expr  = "match" expression NEWLINE INDENT { arm } DEDENT ;
arm         = pattern "->" ( expression NEWLINE | print_stmt | return_stmt | NEWLINE block ) ;
pattern     = "_" | "none" | literal
            | NAME [ "(" [ NAME { "," NAME } ] ")" ]
            | NAME ":" type ;
literal     = INT | FLOAT | TEXT | "true" | "false" ;

return_stmt = "return" ( NEWLINE | value ) ;
print_stmt  = "print" expression NEWLINE ;
expect_stmt = "expect" expression NEWLINE ;
unsafe_stmt = "unsafe" NEWLINE block ;

expr_stmt   = expression [ assign_op expression ] NEWLINE ;
assign_op   = "=" | "+=" | "-=" | "*=" | "/=" ;
```

The target of an assignment must be a name, field, or index expression. The parser checks this. A `match` used as a value cannot have block arms.

## Expressions

Precedence, lowest first:

| Level | Operators | Associativity |
| --- | --- | --- |
| 1 | `x => y`, `if a then b else c` | right |
| 2 | `else` | right |
| 3 | `or` | left |
| 4 | `and` | left |
| 5 | `not` | prefix |
| 6 | `==` `!=` `<` `<=` `>` `>=` `is` `is not` `in` `not in` | none (no chaining) |
| 7 | `..` `..=` | none |
| 8 | `+` `-` | left |
| 9 | `*` `/` `//` `%` | left |
| 10 | `-` `try` `spawn` `shared` `weak` | prefix |
| 11 | call, index, field | left |

```ebnf
expression  = lambda | if_expr | fallback ;
lambda      = ( NAME | "(" [ NAME { "," NAME } ] ")" ) "=>" expression ;
if_expr     = "if" expression "then" logic_or "else" expression ;
fallback    = logic_or [ "else" expression ] ;
logic_or    = logic_and { "or" logic_and } ;
logic_and   = logic_not { "and" logic_not } ;
logic_not   = "not" logic_not | compare ;
compare     = range [ compare_op range | "is" [ "not" ] is_target ] ;
compare_op  = "==" | "!=" | "<" | "<=" | ">" | ">=" | "in" | "not" "in" ;
is_target   = "none" | "empty" | type_term ;
range       = sum [ ( ".." | "..=" ) sum ] ;
sum         = product { ( "+" | "-" ) product } ;
product     = unary { ( "*" | "/" | "//" | "%" ) unary } ;
unary       = ( "-" | "try" | "spawn" | "shared" | "weak" ) unary | postfix ;
postfix     = primary { "(" [ args ] ")" | "[" expression { "," expression } "]" | "." NAME } ;
args        = arg { "," arg } [ "," ] ;
arg         = [ "change" ] expression | NAME ":" expression ;
primary     = NAME | "self" | "none" | "true" | "false" | INT | FLOAT | TEXT
            | "(" expression ")"
            | "[" [ expression { "," expression } [ "," ] ] "]"
            | "{" [ entry { "," entry } [ "," ] ] "}" ;
entry       = expression ":" expression ;
```

## Disambiguation notes

- **`then` branch:** it uses `logic_or`, not `expression`, so the `else` in `if a then b else c` is never read as a fallback.
- **`is` target:** it is a single `type_term`, so `or` after it stays boolean: `x is text or y > 3`.
- **`try` scope:** `try` covers the whole postfix chain. `try read(p).trim()` calls `.trim()` first, on the union, and fails type checking. Write `(try read(p)).trim()`.
- **Generic calls:** `f[int](x)` parses as an index followed by a call. The resolver rewrites it to a generic instantiation once it knows `f` is a task.
- **Lambdas:** recognize with one token of lookahead past `NAME`, or by scanning to the matching `)` and checking for `=>`.
