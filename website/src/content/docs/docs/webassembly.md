---
title: WebAssembly
description: Build Lucid programs for the browser and Node, and write interactive pages with std.dom.
---

`lucid build --target web` compiles a program to WebAssembly. The output is one file, `NAME.js`, which holds a small loader and two WebAssembly modules: the program and its runtime.

```sh
lucid build --target web hello.lcd   # writes hello.js
node hello.js                        # runs it under Node
```

A page loads it with one tag:

```html
<!doctype html>
<html lang="en">
  <head><meta charset="utf-8"><title>Counter</title></head>
  <body>
    <script src="counter.js"></script>
  </body>
</html>
```

Every built feature works on the web, except those a page cannot have:

| Feature | On the web |
| --- | --- |
| `spawn`, `std.net`, `std.http`, `std.crypto`, `std.sql` | Error E0907 |
| `extern` tasks and `.pointer()` | Error E0905 |
| `std.fs` | Gives `Failed` |
| `env.get`, `args.all()` | `none`, `[]` |
| `time.sleep` | Traps |
| Channels and locks | Work, with one thread |

## Pages with std.dom

`std.dom` builds pages in the Elm architecture:

- a **model**: any value that holds the page's state,
- `view(model)`: gives a tree of `dom.Node[Msg]`, where each handler gives a message,
- `update(model, message)`: gives the next model,
- `dom.app(init, view, update)`: shows `view(init)`, and after each event runs `update` and then `view`, and changes only the nodes that differ.

```lucid title="counter.lcd"
use std.dom

choice Message
    Increment
    Decrement

task update(count: int, message: Message) -> int
    return match message
        Increment -> count + 1
        Decrement -> count - 1

task button(label: text, message: Message) -> dom.Node[Message]
    return dom.Element(tag: "button", attrs: [dom.On(event: "click", message: _ => message)], children: [dom.Text(content: label)])

task view(count: int) -> dom.Node[Message]
    return dom.Element(tag: "p", children: [button("-", Decrement), dom.Text(content: " {count} "), button("+", Increment)])

dom.app(0, view, update)
```

| Constructor | Makes |
| --- | --- |
| `dom.Element(tag:, attrs: [], children: [])` | An element |
| `dom.Text(content:)` | Text, never read as HTML |
| `dom.Attribute(name:, value:)` | An attribute. `value` and `checked` also set an input's state |
| `dom.On(event:, message:)` | A handler. `message` gets the event's key, or the element's value |

`std.dom` works only with `--target web`. Elsewhere it is error E0906.

## Safe by construction

`std.dom` never reads text as HTML. It traps on a `script` element, an attribute whose name starts with `on`, `formaction`, `srcdoc`, and a `javascript:` or `data:text/html` value. So a page runs only code that the program wrote, even when values come from a user.

Hash keys on the web come from `crypto.getRandomValues`, as they come from the system on native, so map order and collisions cannot be planned by whoever supplies the keys.
