"use strict";
// Made by `lucid build --target web`. Runs a Lucid program in a browser, from
// <script src>, or under `node`. See "WebAssembly" in docs/compiler.md.
(async () => {
    const RUNTIME = "@RUNTIME@";
    const PROGRAM = "@PROGRAM@";
    const DATA = @DATA@;
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
    const js = {
        write: (stream, at, len) => write(stream, bytes(at, len)),
        trap: (at, len) => {
            write(2, bytes(at, len));
            write(2, text("\n"));
            throw new Stop(101);
        },
        exit: code => { throw new Stop(code); },
        now: () => Date.now(),
        clock: () => performance.now(),
        random: (at, len) => { crypto.getRandomValues(bytes(at, len)); },
    };

    let code = 0;
    try {
        const runtime = (await WebAssembly.instantiate(decode(RUNTIME), { js })).instance.exports;
        memory = runtime.memory;
        // Static data goes in pages of its own, past what the runtime uses.
        const base = memory.grow(Math.ceil(DATA / 65536)) * 65536;
        const env = { memory_base: new WebAssembly.Global({ value: "i32" }, base) };
        const program = await WebAssembly.instantiate(decode(PROGRAM), { rt: runtime, env });
        program.instance.exports.main();
    } catch (error) {
        if (!(error instanceof Stop)) {
            write(2, text(`trap: ${error}\n`));
        }
        code = error instanceof Stop ? error.code : 101;
    }
    for (const stream of [1, 2]) {
        if (!node && partial[stream] !== "") {
            show(stream, partial[stream]);
        }
    }
    if (node) {
        process.exitCode = code;
    }
})();
