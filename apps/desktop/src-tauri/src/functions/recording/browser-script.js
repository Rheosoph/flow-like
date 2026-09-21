(() => {
  globalThis.__flowLikeRecordingCleanup?.();
  const controller = new AbortController();
  globalThis.__flowLikeRecordingCleanup = () => {
    controller.abort();
    for (let index = 0; index < window.length; index++) {
      try { window[index].__flowLikeRecordingCleanup?.(); } catch (_) {}
    }
  };
  const selector = (element) => {
    if (element?.nodeType !== 1) throw new Error("The event has no element locator.");
    const doc = element.ownerDocument;
    const unique = (value) => doc.querySelectorAll(value).length === 1;
    if (element.id && unique(`#${CSS.escape(element.id)}`)) return `#${CSS.escape(element.id)}`;
    for (const attribute of ["data-testid", "data-test", "name", "aria-label"]) {
      const value = element.getAttribute(attribute);
      if (value) {
        const candidate = `${element.localName}[${attribute}=${JSON.stringify(value)}]`;
        if (unique(candidate)) return candidate;
      }
    }
    const parts = [];
    for (let current = element; current; current = current.parentElement) {
      const siblings = current.parentElement ? [...current.parentElement.children].filter((child) => child.localName === current.localName) : [current];
      parts.unshift(`${current.localName}:nth-of-type(${siblings.indexOf(current) + 1})`);
      const candidate = parts.join(" > ");
      if (unique(candidate)) return candidate;
    }
    throw new Error("This element needs a shadow DOM locator; its action was not recorded.");
  };
  const send = (kind, element, value = "", event = {}) => {
    try {
      const frames = [];
      let frame = window;
      while (frame !== frame.top) {
        if (!frame.frameElement) throw new Error("Recording inside a cross-origin frame requires a frame locator. Its action was not recorded.");
        frames.unshift(selector(frame.frameElement));
        frame = frame.parent;
      }
      const modifiers = [];
      if (event.shiftKey) modifiers.push("Shift");
      if (event.ctrlKey) modifiers.push("Control");
      if (event.altKey) modifiers.push("Alt");
      if (event.metaKey) modifiers.push("Meta");
      const payload = { kind, selector: selector(element), value, url: frame.location.href, frames, button: event.button === 2 ? "Right" : event.button === 1 ? "Middle" : "Left", modifiers, scroll_x: element?.scrollLeft ?? 0, scroll_y: element?.scrollTop ?? 0 };
      globalThis.__flowLikeRecorder(JSON.stringify({ action: payload, timestamp: Date.now() }));
    } catch (error) {
      globalThis.__flowLikeRecorder(JSON.stringify({ error: error.message }));
    }
  };
  const on = (type, listener) => document.addEventListener(type, (event) => {
    if (event.isTrusted) listener(event);
  }, { capture: true, signal: controller.signal });
  on("click", (event) => { if (event.detail < 2) send("click", event.composedPath()[0], "", event); });
  on("auxclick", (event) => { if (event.button === 1) send("click", event.composedPath()[0], "", event); });
  on("contextmenu", (event) => send("click", event.composedPath()[0], "", event));
  on("dblclick", (event) => send("double_click", event.composedPath()[0], "", event));
  on("input", (event) => {
    const element = event.composedPath()[0];
    if (element instanceof HTMLSelectElement) return;
    if (element.type === "password" || element.type === "file") {
      globalThis.__flowLikeRecorder(JSON.stringify({ error: "Password and file inputs need explicit workflow inputs. Their contents were not recorded." }));
      return;
    }
    if (element instanceof HTMLInputElement && ["checkbox", "radio", "range", "color"].includes(element.type)) return;
    send("type", element, element.value ?? element.textContent ?? "");
  });
  on("change", (event) => {
    const element = event.composedPath()[0];
    if (element instanceof HTMLSelectElement) send("select", element, element.value);
  });
  on("keydown", (event) => {
    const stopModifier = /Mac|iPhone|iPad/.test(navigator.platform) ? event.metaKey : event.ctrlKey;
    if (stopModifier && event.shiftKey && event.key.toLowerCase() === "s") {
      event.preventDefault();
      event.stopImmediatePropagation();
      globalThis.__flowLikeRecorder(JSON.stringify({ stop: true }));
      return;
    }
    if ((event.ctrlKey || event.metaKey) && !event.altKey && ["a", "v", "x", "z", "y"].includes(event.key.toLowerCase())) return;
    const special = ["Enter", "Tab", "Escape", "ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight", "Home", "End", "PageUp", "PageDown"].includes(event.key);
    const shortcut = (event.ctrlKey || event.metaKey) && !event.altKey && event.key.length === 1;
    if (special || shortcut) send("key", event.composedPath()[0], event.key, event);
  });
  let scrollTimer;
  on("scroll", (event) => {
    clearTimeout(scrollTimer);
    const element = event.target instanceof Element ? event.target : document.scrollingElement;
    scrollTimer = setTimeout(() => send("scroll", element), 80);
  });
  controller.signal.addEventListener("abort", () => clearTimeout(scrollTimer));
})();
