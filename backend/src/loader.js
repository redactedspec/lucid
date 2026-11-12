"use strict";
// Made by `lucid build --target web`. Runs a Lucid program in a browser, from
// <script src>, or under `node`. See "WebAssembly" in docs/compiler.md.
(async () => {
    const RUNTIME = "@RUNTIME@";
    const PROGRAM = "@PROGRAM@";
    const DATA = @DATA@;
    // How many tasks the program adds to the runtime's table.
    const TABLE = @TABLE@;
    // Each call between tasks, by "function:offset", with what a trace shows
    // for it, or "" for a call that a trace leaves out.
    const SITES = @SITES@;
    // Where each task starts, by function, for a stack overflow.
    const TASKS = @TASKS@;
    // A trace shows 16 calls, as on native, after some frames of the runtime.
    const MAX_TRACE = 16;
    if (Error.stackTraceLimit < 64) {
        Error.stackTraceLimit = 64;
    }
    const node = typeof process === "object" && Boolean(process.versions?.node);
    const decode = text => node
        ? Buffer.from(text, "base64")
        : Uint8Array.from(atob(text), c => c.charCodeAt(0));

    // Thrown to end the program: a trap or an exit never returns to the module.
    class Stop {
        constructor(code) { this.code = code; }
    }

    let memory;
    const bytes = (at, len) => new Uint8Array(memory.buffer, at, len);
    // A browser's console shows whole lines, so each stream keeps a partial one.
    const decoders = [null, new TextDecoder(), new TextDecoder()];
    const partial = ["", "", ""];
    const show = (stream, line) => stream === 1 ? console.log(line) : console.error(line);
    const write = (stream, data) => {
        if (node) {
            require("fs").writeSync(stream, data);
            return;
        }
        const lines = (partial[stream] + decoders[stream].decode(data, { stream: true })).split("\n");
        partial[stream] = lines.pop();
        lines.forEach(line => show(stream, line));
    };
    const text = message => new TextEncoder().encode(message);
    // The WebAssembly frames of a stack, innermost first: each program frame
    // as its function and "function:offset", and a runtime frame as null.
    const frames = stack => stack.split("\n").flatMap(line => {
        const found = line.match(/wasm-function\[(\d+)\]:0x([0-9a-f]+)/);
        if (!found) {
            return [];
        }
        return [line.includes("lucid_rt") ? null : { task: found[1], site: `${found[1]}:${parseInt(found[2], 16)}` }];
    });
    // The first program frame, which the message places, and a trace of the
    // calls under it. As on native, the trace ends at a frame of the runtime.
    const place = stack => {
        const all = frames(stack);
        const first = all.findIndex(frame => frame !== null);
        let trace = "";
        let shown = 0;
        for (const frame of all.slice(first + 1)) {
            const site = frame === null ? undefined : SITES[frame.site];
            if (site === undefined) {
                break;
            }
            if (site === "") {
                continue;
            }
            if (shown === MAX_TRACE) {
                trace += "\n    ...";
                break;
            }
            trace += `\n    at ${site}`;
            shown += 1;
        }
        return { task: first < 0 ? undefined : all[first].task, trace };
    };
    const js = {
        write: (stream, at, len) => write(stream, bytes(at, len)),
        trap: (at, len) => {
            write(2, bytes(at, len));
            write(2, text(`${place(new Error().stack).trace}\n`));
            throw new Stop(101);
        },
        exit: code => { throw new Stop(code); },
        now: () => Date.now(),
        clock: () => performance.now(),
        random: (at, len) => { crypto.getRandomValues(bytes(at, len)); },
        dom_create: (at, len) => add(document.createElement(read(at, len))),
        dom_text: (at, len) => add(document.createTextNode(read(at, len))),
        dom_set_text: (node, at, len) => { nodes.get(node).data = read(at, len); },
        dom_set: (node, at, len, value, valueLen) => {
            const element = nodes.get(node);
            const name = read(at, len);
            element.setAttribute(name, read(value, valueLen));
            // The attribute is only where an <input> starts: typing or a click changes the property.
            if (name === "value") {
                element.value = read(value, valueLen);
            } else if (name === "checked") {
                element.checked = true;
            }
        },
        dom_unset: (node, at, len) => {
            const element = nodes.get(node);
            const name = read(at, len);
            element.removeAttribute(name);
            if (name === "checked") {
                element.checked = false;
            }
        },
        dom_on: (node, at, len, handler) => {
            const element = nodes.get(node);
            const name = read(at, len);
            element.lucidOn ??= {};
            if (!(name in element.lucidOn)) {
                element.addEventListener(name, happened => fire(element, name, happened));
            }
            element.lucidOn[name] = handler;
        },
        dom_off: (node, at, len) => { nodes.get(node).lucidOn[read(at, len)] = undefined; },
        dom_append: (parent, child) => (parent === 0 ? document.body : nodes.get(parent)).appendChild(nodes.get(child)),
        dom_replace: (old, node) => {
            nodes.get(old).parentNode.replaceChild(nodes.get(node), nodes.get(old));
            forget(nodes.get(old));
        },
        dom_remove: node => {
            nodes.get(node).parentNode.removeChild(nodes.get(node));
            forget(nodes.get(node));
        },
        dom_value_len: () => value.length,
        dom_value: at => bytes(at, value.length).set(value),
    };

    // The page's nodes, by the numbers that `std.dom` holds. 0 is the page itself.
    const nodes = new Map();
    let last = 0;
    const read = (at, len) => new TextDecoder().decode(bytes(at, len));
    const add = node => {
        last += 1;
        node.lucid = last;
        nodes.set(last, node);
        return last;
    };
    const forget = node => {
        nodes.delete(node.lucid);
        node.lucidOn = undefined;
        node.childNodes.forEach(forget);
    };
    // What a handler gets: the key of a key event, or else the value of the
    // node it happened on, as an <input> has, or else "".
    let value = new Uint8Array();
    // A change to the page can start an event at once, as removing a focused
    // node starts "blur". Such an event waits for the handler that runs now,
    // and then finds its handler's number in the page that handler left.
    const waiting = [];
    const fire = (element, name, happened) => {
        const key = happened.key;
        waiting.push([element, name, text(typeof key === "string" ? key : String(happened.target?.value ?? ""))]);
        if (waiting.length > 1) {
            return;
        }
        while (waiting.length > 0 && !stopped) {
            const [element, name, given] = waiting[0];
            const handler = element.lucidOn?.[name];
            value = given;
            try {
                if (handler !== undefined) {
                    runtime.lucid_dom_fire(handler);
                }
                end();
            } catch (error) {
                end(error);
            }
            waiting.shift();
        }
        waiting.length = 0;
    };

    let code = 0;
    let stopped = false;
    let runtime;
    // Reports an error that ended the program, then shows what is left of the
    // output. After an error, the page takes no more events.
    const end = error => {
        if (error !== undefined) {
            // V8 throws a RangeError when the stack runs out, and Firefox an
            // InternalError, "too much recursion".
            const overflow = error instanceof RangeError || error?.name === "InternalError";
            const { task, trace } = overflow ? place(error.stack) : {};
            if (task !== undefined) {
                write(2, text(`${TASKS[task]}: trap: stack overflow${trace}\n`));
            } else if (!(error instanceof Stop)) {
                write(2, text(`trap: ${error}\n`));
            }
            code = error instanceof Stop ? error.code : 101;
            stopped = true;
        }
        for (const stream of [1, 2]) {
            if (!node && partial[stream] !== "") {
                show(stream, partial[stream]);
                partial[stream] = "";
            }
        }
        if (node) {
            process.exitCode = code;
        }
    };

    try {
        runtime = (await WebAssembly.instantiate(decode(RUNTIME), { js })).instance.exports;
        memory = runtime.memory;
        // Static data goes in pages of its own, past what the runtime uses.
        const base = memory.grow(Math.ceil(DATA / 65536)) * 65536;
        // The program's tasks go in the table too, so the runtime can call them.
        const tableBase = runtime.__indirect_function_table.grow(TABLE);
        const env = {
            memory_base: new WebAssembly.Global({ value: "i32" }, base),
            table_base: new WebAssembly.Global({ value: "i32" }, tableBase),
        };
        const program = await WebAssembly.instantiate(decode(PROGRAM), { rt: runtime, env });
        program.instance.exports.main();
        end();
    } catch (error) {
        end(error);
    }
})();
