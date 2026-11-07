"use strict";
// A small fake `document` for the `std.dom` tests, by decision 164. Usage:
// `node page.js NAME.js NAME.events`. It loads the built program, then runs
// each line of the events file once the page is shown, and prints the line
// and then the page. A line is `click ID`, `input ID TEXT`, `key ID KEY`,
// `check ID`, which flips a checkbox and fires "change", or `focus ID`,
// where ID is an element's `id` attribute. As in a browser, a focused
// element that leaves the page gets "blur" at once. After each line it also
// prints how many nodes the program made, so a test shows that a patch keeps
// nodes.
const fs = require("fs");
const path = require("path");

class Node {
    constructor() {
        this.childNodes = [];
        this.parentNode = null;
    }
    appendChild(child) {
        child.parentNode?.removeChild(child);
        child.parentNode = this;
        this.childNodes.push(child);
        return child;
    }
    removeChild(child) {
        this.childNodes.splice(this.childNodes.indexOf(child), 1);
        child.parentNode = null;
        blur(child);
        return child;
    }
    replaceChild(child, old) {
        child.parentNode?.removeChild(child);
        this.childNodes[this.childNodes.indexOf(old)] = child;
        child.parentNode = this;
        old.parentNode = null;
        blur(old);
        return old;
    }
}

class Element extends Node {
    constructor(tag) {
        super();
        this.tag = tag;
        this.attributes = new Map();
        this.listeners = new Map();
        this.value = "";
        this.checked = false;
    }
    setAttribute(name, value) {
        this.attributes.set(name, String(value));
    }
    removeAttribute(name) {
        this.attributes.delete(name);
    }
    addEventListener(name, listener) {
        if (this.listeners.has(name)) {
            throw new Error(`a second listener for ${name}`);
        }
        this.listeners.set(name, listener);
    }
}

class Text extends Node {
    constructor(data) {
        super();
        this.data = data;
    }
}

let focused;
const holds = (node, inner) => node === inner || node.childNodes.some(child => holds(child, inner));
const blur = gone => {
    if (focused !== undefined && holds(gone, focused)) {
        const target = focused;
        focused = undefined;
        target.listeners.get("blur")?.({ target });
    }
};

let shown;
const mounted = new Promise(resolve => { shown = resolve; });
const body = new Element("body");
const bodyAppend = body.appendChild.bind(body);
body.appendChild = child => {
    bodyAppend(child);
    shown();
    return child;
};
let made = 0;
globalThis.document = {
    body,
    createElement: tag => { made += 1; return new Element(tag); },
    createTextNode: data => { made += 1; return new Text(data); },
};

const html = node => {
    if (node instanceof Text) {
        return node.data;
    }
    const attributes = [...node.attributes].map(([name, value]) => ` ${name}="${value}"`).join("");
    const value = node.tag === "input" ? ` .value="${node.value}"${node.checked ? " .checked" : ""}` : "";
    return `<${node.tag}${attributes}${value}>${node.childNodes.map(html).join("")}</${node.tag}>`;
};
const find = (node, id) => {
    if (node instanceof Element && node.attributes.get("id") === id) {
        return node;
    }
    for (const child of node.childNodes) {
        const found = find(child, id);
        if (found) {
            return found;
        }
    }
    return undefined;
};
const say = line => fs.writeSync(1, `${line}\n`);

const [program, events] = process.argv.slice(2);
require(path.resolve(program));
mounted.then(() => {
    say(html(body));
    for (const line of fs.readFileSync(events, "utf8").split("\n").filter(Boolean)) {
        const [kind, id, ...rest] = line.trim().split(" ");
        const target = find(body, id);
        say(`> ${line.trim()}`);
        if (target === undefined) {
            say("no such element");
            continue;
        }
        const listener = target.listeners.get({ key: "keydown", check: "change" }[kind] ?? kind);
        if (kind === "input") {
            target.value = rest.join(" ");
        } else if (kind === "check") {
            target.checked = !target.checked;
        }
        made = 0;
        if (kind === "focus") {
            focused = target;
        }
        listener?.(kind === "key" ? { key: rest.join(" "), target } : { target });
        say(`${html(body)} made ${made}`);
    }
});
