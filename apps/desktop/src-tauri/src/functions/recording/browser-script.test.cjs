const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");
const vm = require("node:vm");

const script = fs.readFileSync(path.join(__dirname, "browser-script.js"), "utf8");

function page() {
  const events = new Map();
  const elements = new Map();
  const payloads = [];
  const document = {
    querySelectorAll: (selector) => [...elements.values()].filter((element) => selector === `#${element.id}`),
    addEventListener(type, callback, { signal }) {
      const listeners = events.get(type) ?? new Set();
      events.set(type, listeners);
      listeners.add(callback);
      signal.addEventListener("abort", () => listeners.delete(callback));
    },
  };
  class Element {
    constructor(id) {
      this.id = id;
      this.nodeType = 1;
      this.ownerDocument = document;
      this.localName = "input";
      this.parentElement = null;
      this.value = "";
      elements.set(id, this);
    }
    getAttribute() { return null; }
  }
  class Input extends Element {}
  class Select extends Element {}
  const window = { location: { href: "https://example.test/form" } };
  window.top = window;
  const context = vm.createContext({
    AbortController, Element, HTMLInputElement: Input, HTMLSelectElement: Select,
    CSS: { escape: (value) => value }, document, window,
    navigator: { platform: "MacIntel" }, setTimeout, clearTimeout,
    __flowLikeRecorder: (payload) => payloads.push(JSON.parse(payload)),
  });
  vm.runInContext(script, context);
  return {
    payloads, context,
    input: (id, type = "text") => Object.assign(new Input(id), { type }),
    dispatch(type, element, properties = {}) {
      const event = { isTrusted: true, target: element, composedPath: () => [element], ...properties };
      for (const callback of events.get(type) ?? []) callback(event);
    },
  };
}

test("trusted Unicode input retains its locator and complete value", () => {
  const browser = page();
  const input = browser.input("name");
  input.value = "Zoë 文 👩‍💻";
  browser.dispatch("input", input);
  assert.equal(browser.payloads[0].action.selector, "#name");
  assert.equal(browser.payloads[0].action.value, input.value);
  browser.dispatch("input", input, { isTrusted: false });
  assert.equal(browser.payloads.length, 1);
});

test("password and file contents never enter recorder payloads", () => {
  const browser = page();
  for (const type of ["password", "file"]) {
    const input = browser.input(type, type);
    input.value = "private-value";
    browser.dispatch("input", input);
  }
  assert.equal(browser.payloads.length, 2);
  assert.ok(browser.payloads.every((payload) => payload.error && !payload.action));
  assert.ok(!JSON.stringify(browser.payloads).includes("private-value"));
});

test("double clicks keep button and modifiers without a duplicate second click", () => {
  const browser = page();
  const input = browser.input("target");
  browser.dispatch("click", input, { detail: 1, button: 1, ctrlKey: true });
  browser.dispatch("click", input, { detail: 2, button: 1, ctrlKey: true });
  browser.dispatch("dblclick", input, { button: 1, ctrlKey: true });
  assert.deepEqual(browser.payloads.map((payload) => payload.action.kind), ["click", "double_click"]);
  assert.equal(browser.payloads[1].action.button, "Middle");
  assert.deepEqual(browser.payloads[1].action.modifiers, ["Control"]);
});

test("reinstall and cleanup remove listeners and pending scroll recording", async () => {
  const browser = page();
  const input = browser.input("target");
  vm.runInContext(script, browser.context);
  browser.dispatch("click", input, { detail: 1 });
  assert.equal(browser.payloads.length, 1);
  browser.dispatch("scroll", input);
  vm.runInContext("globalThis.__flowLikeRecordingCleanup()", browser.context);
  browser.dispatch("click", input, { detail: 1 });
  await new Promise((resolve) => setTimeout(resolve, 100));
  assert.equal(browser.payloads.length, 1);
});

test("the browser stop chord calls the backend binding", () => {
  const browser = page();
  let prevented = false;
  browser.dispatch("keydown", browser.input("target"), {
    key: "s", metaKey: true, shiftKey: true,
    preventDefault() { prevented = true; }, stopImmediatePropagation() {},
  });
  assert.equal(prevented, true);
  assert.deepEqual(browser.payloads, [{ stop: true }]);
});

test("same-origin frame locators work across different DOM realms", () => {
  const browser = page();
  const parent = { location: { href: "https://example.test/parent" } };
  parent.top = parent;
  const frame = {
    nodeType: 1, id: "child-frame",
    ownerDocument: { querySelectorAll: (selector) => selector === "#child-frame" ? [frame] : [] },
  };
  Object.assign(browser.context.window, { top: parent, parent, frameElement: frame });
  browser.dispatch("click", browser.input("target"), { detail: 1 });
  assert.deepEqual(browser.payloads[0].action.frames, ["#child-frame"]);
  assert.equal(browser.payloads[0].action.url, parent.location.href);
});

test("modified submit and navigation keys preserve modifiers and the target", () => {
  const browser = page();
  browser.dispatch("keydown", browser.input("target"), { key: "Enter", ctrlKey: true, shiftKey: true });
  assert.equal(browser.payloads[0].action.kind, "key");
  assert.equal(browser.payloads[0].action.value, "Enter");
  assert.equal(browser.payloads[0].action.selector, "#target");
  assert.deepEqual(browser.payloads[0].action.modifiers, ["Shift", "Control"]);
});

test("edit shortcuts replay only their resulting value without repeating clipboard or undo actions", () => {
  for (const key of ["a", "v", "x", "z", "y"]) {
    for (const modifier of ["ctrlKey", "metaKey"]) {
      const browser = page();
      const input = browser.input("target");
      browser.dispatch("keydown", input, { key, [modifier]: true });
      input.value = "result after edit";
      browser.dispatch("input", input);
      assert.equal(browser.payloads.length, 1, `${modifier}+${key}`);
      assert.equal(browser.payloads[0].action.kind, "type");
      assert.equal(browser.payloads[0].action.value, input.value);
    }
  }
});
