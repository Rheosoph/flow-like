import {
	DEFAULT_DARK_TOKENS,
	DEFAULT_LIGHT_TOKENS,
	FLW_PROTOCOL,
} from "@flow-like/widget-sdk";
import { escapeHtmlAttr } from "../html";

export interface HarnessWidget {
	group: string;
	id: string;
	/** Full child dev-server document URL for the sandboxed iframe */
	entryUrl: string;
}

interface HarnessData {
	protocol: string;
	widgets: HarnessWidget[];
	themes: {
		light: Record<string, string>;
		dark: Record<string, string>;
	};
}

function embedJson(data: HarnessData): string {
	return JSON.stringify(data).replace(/</g, "\\u003c");
}

function sidebarMarkup(widgets: HarnessWidget[]): string {
	const byGroup = new Map<string, HarnessWidget[]>();
	for (const widget of widgets) {
		const list = byGroup.get(widget.group) ?? [];
		list.push(widget);
		byGroup.set(widget.group, list);
	}
	const parts: string[] = [];
	for (const group of [...byGroup.keys()].sort()) {
		parts.push(`<div class="group-name">${escapeHtmlAttr(group)}</div>`);
		for (const widget of byGroup.get(group) ?? []) {
			parts.push(
				`<button type="button" class="widget-item" data-key="${escapeHtmlAttr(
					`${widget.group}/${widget.id}`,
				)}">${escapeHtmlAttr(widget.id)}</button>`,
			);
		}
	}
	return parts.join("\n");
}

const HARNESS_CSS = `
:root {
	--h-bg: #f6f6f7; --h-fg: #1c1c1f; --h-panel: #ffffff; --h-border: #d9d9de;
	--h-muted: #6b6b74; --h-accent: #e05d38; --h-ok: #1a7f37; --h-err: #c62828;
	--h-warn: #9a6700; --h-mono: ui-monospace, "JetBrains Mono", Menlo, monospace;
}
body.dark {
	--h-bg: #17181c; --h-fg: #e8e8ea; --h-panel: #1f2026; --h-border: #34353d;
	--h-muted: #9a9aa4; --h-ok: #4caf72; --h-err: #ef6a6a; --h-warn: #d4a72c;
}
* { box-sizing: border-box; }
html, body { height: 100%; }
body {
	margin: 0; font: 13px/1.45 system-ui, sans-serif;
	background: var(--h-bg); color: var(--h-fg);
}
button, select, input, textarea {
	font: inherit; color: inherit; background: var(--h-panel);
	border: 1px solid var(--h-border); border-radius: 4px; padding: 3px 8px;
}
button { cursor: pointer; }
button:hover { border-color: var(--h-accent); }
#app { display: flex; height: 100vh; }
#sidebar {
	width: 220px; flex: none; border-right: 1px solid var(--h-border);
	background: var(--h-panel); padding: 12px; overflow-y: auto;
}
#sidebar h1 { font-size: 14px; margin: 0 0 12px; }
#sidebar h1 span { color: var(--h-muted); font-weight: normal; }
.group-name {
	font-size: 11px; text-transform: uppercase; letter-spacing: 0.05em;
	color: var(--h-muted); margin: 10px 0 4px;
}
.widget-item {
	display: block; width: 100%; text-align: left; border: none;
	background: none; padding: 5px 8px; border-radius: 4px; margin: 1px 0;
}
.widget-item:hover { background: var(--h-bg); }
.widget-item.active { background: var(--h-accent); color: #fff; }
#main { flex: 1; display: flex; flex-direction: column; min-width: 0; }
#toolbar {
	display: flex; align-items: center; gap: 10px; flex-wrap: wrap;
	padding: 8px 12px; border-bottom: 1px solid var(--h-border);
	background: var(--h-panel);
}
#widget-title { font-weight: 600; }
#status { color: var(--h-muted); }
#status.ok { color: var(--h-ok); }
#status.err { color: var(--h-err); }
.spacer { flex: 1; }
.control { display: inline-flex; align-items: center; gap: 5px; color: var(--h-muted); }
#height-display { font-family: var(--h-mono); font-size: 12px; }
#content { flex: 1; display: flex; min-height: 0; }
#stage-wrap { flex: 1; overflow: auto; padding: 16px; }
#stage { width: 100%; margin: 0 auto; transition: width 0.15s ease; }
#stage iframe {
	width: 100%; border: 1px dashed var(--h-border); border-radius: 6px;
	background: #fff; display: block;
}
body.dark #stage iframe { background: #17181c; }
#panels {
	width: 380px; flex: none; overflow-y: auto; padding: 10px;
	border-left: 1px solid var(--h-border); background: var(--h-panel);
}
#panels details { border-bottom: 1px solid var(--h-border); padding: 6px 0; }
#panels summary {
	cursor: pointer; font-weight: 600; padding: 4px 0;
	display: flex; align-items: center; gap: 8px;
}
#panels summary button { font-size: 11px; padding: 1px 6px; margin-left: auto; }
.empty { color: var(--h-muted); padding: 4px 0; }
.field { margin: 8px 0; }
.field-label { display: block; font-weight: 600; margin-bottom: 2px; }
.field-desc { color: var(--h-muted); font-size: 12px; margin-bottom: 3px; }
.field input[type="text"], .field input[type="number"], .field select,
.field textarea { width: 100%; }
.field textarea { font-family: var(--h-mono); font-size: 12px; }
.field textarea.bad { border-color: var(--h-err); }
.field-error { color: var(--h-err); font-size: 12px; min-height: 0; }
.fixture-btn { margin: 2px 4px 2px 0; }
#event-log, #trace {
	max-height: 220px; overflow-y: auto; font-family: var(--h-mono);
	font-size: 11px; background: var(--h-bg); border-radius: 4px; padding: 6px;
}
.log-entry { border-bottom: 1px solid var(--h-border); padding: 3px 0; }
.log-entry.invalid .log-name { color: var(--h-err); }
.log-entry.meta .log-name { color: var(--h-muted); }
.log-head { display: flex; gap: 8px; align-items: baseline; }
.log-time { color: var(--h-muted); }
.log-name { font-weight: 600; }
.log-flag {
	color: #fff; background: var(--h-err); border-radius: 3px;
	padding: 0 4px; font-size: 10px;
}
.log-payload { margin: 2px 0; white-space: pre-wrap; word-break: break-all; }
.log-error { color: var(--h-err); }
.trace-line { white-space: pre-wrap; word-break: break-all; }
.trace-in { color: var(--h-ok); }
.trace-out { color: var(--h-accent); }
.trace-note { color: var(--h-warn); }
#query-panel { display: flex; flex-direction: column; gap: 6px; padding: 6px 0; }
#query-args { font-family: var(--h-mono); font-size: 12px; }
#query-result {
	font-family: var(--h-mono); font-size: 12px; white-space: pre-wrap;
	word-break: break-all; margin: 0; min-height: 18px;
}
#query-result.ok { color: var(--h-ok); }
#query-result.err { color: var(--h-err); }
#query-result.warn { color: var(--h-warn); }
.token-row { display: flex; gap: 6px; align-items: center; margin: 3px 0; }
.token-name { font-family: var(--h-mono); font-size: 11px; flex: none; width: 150px; }
.token-row input { flex: 1; font-family: var(--h-mono); font-size: 11px; }
`;

// The page script is plain browser JS (no framework, no external assets). It
// is the real flw/1 host: nonce handshake, init on hello/load, props:update,
// theme:change, query round-trips, event/resize/value:changed handling.
// No template literals inside — the file itself is a TS template literal.
const HARNESS_JS = `
"use strict";
(function () {
	var H = window.__HARNESS__;
	var TRACE_CAP = 300;
	var LOG_CAP = 200;

	var state = {
		widgets: H.widgets.slice(),
		selected: null,
		contract: null,
		formModel: [],
		fixtures: {},
		props: {},
		session: null,
		mode: "light",
		overrides: { light: {}, dark: {} },
		preview: false,
		pending: {}
	};

	function $(id) { return document.getElementById(id); }

	function make(tag, className, text) {
		var node = document.createElement(tag);
		if (className) node.className = className;
		if (text !== undefined) node.textContent = text;
		return node;
	}

	function uid() {
		return window.crypto && crypto.randomUUID
			? crypto.randomUUID()
			: "id-" + Date.now().toString(36) + "-" + Math.random().toString(36).slice(2);
	}

	function timestamp() {
		var d = new Date();
		function pad(n, w) { return String(n).padStart(w || 2, "0"); }
		return pad(d.getHours()) + ":" + pad(d.getMinutes()) + ":" +
			pad(d.getSeconds()) + "." + pad(d.getMilliseconds(), 3);
	}

	function shortJson(value) {
		var text;
		try { text = JSON.stringify(value); } catch (e) { text = String(value); }
		if (text === undefined) text = "undefined";
		return text.length > 600 ? text.slice(0, 600) + "\\u2026" : text;
	}

	function appendCapped(container, node, cap) {
		container.appendChild(node);
		while (container.childNodes.length > cap) container.removeChild(container.firstChild);
		container.scrollTop = container.scrollHeight;
	}

	function trace(dir, type, payload) {
		var line = make("div", "trace-line " + (dir === "in" ? "trace-in" : "trace-out"),
			timestamp() + " " + (dir === "in" ? "\\u2190" : "\\u2192") + " " + type + " " + shortJson(payload));
		appendCapped($("trace"), line, TRACE_CAP);
	}

	function traceNote(text) {
		appendCapped($("trace"), make("div", "trace-line trace-note", timestamp() + " \\u00b7 " + text), TRACE_CAP);
	}

	// ---------- minimal contract schema validation (mirrors the SDK subset) ----------
	function typeOk(t, v) {
		switch (t) {
			case "string": return typeof v === "string";
			case "number": return typeof v === "number";
			case "integer": return typeof v === "number" && v % 1 === 0;
			case "boolean": return typeof v === "boolean";
			case "null": return v === null;
			case "array": return Array.isArray(v);
			case "object": return typeof v === "object" && v !== null && !Array.isArray(v);
			default: return true;
		}
	}

	function checkSchema(schema, value, path, errors) {
		if (!schema || typeof schema !== "object") return errors;
		var t = schema.type;
		if (typeof t === "string" && !typeOk(t, value)) {
			errors.push(path + ": expected " + t + ", got " + (value === null ? "null" : Array.isArray(value) ? "array" : typeof value));
			return errors;
		}
		if (Array.isArray(t) && !t.some(function (x) { return typeOk(x, value); })) {
			errors.push(path + ": expected one of [" + t.join(", ") + "]");
			return errors;
		}
		if (Array.isArray(schema.enum) && !schema.enum.some(function (x) { return JSON.stringify(x) === JSON.stringify(value); })) {
			errors.push(path + ": not one of the allowed enum values");
		}
		if ("const" in schema && JSON.stringify(schema.const) !== JSON.stringify(value)) {
			errors.push(path + ": value does not equal const");
		}
		if (typeof value === "number") {
			if (typeof schema.minimum === "number" && value < schema.minimum) errors.push(path + ": " + value + " < minimum " + schema.minimum);
			if (typeof schema.maximum === "number" && value > schema.maximum) errors.push(path + ": " + value + " > maximum " + schema.maximum);
		}
		if (Array.isArray(value) && schema.items && !Array.isArray(schema.items)) {
			value.forEach(function (item, i) { checkSchema(schema.items, item, path + "[" + i + "]", errors); });
		}
		if (value && typeof value === "object" && !Array.isArray(value)) {
			(Array.isArray(schema.required) ? schema.required : []).forEach(function (key) {
				if (!(key in value)) errors.push(path + ": missing required property '" + key + "'");
			});
			var props = schema.properties || {};
			Object.keys(props).forEach(function (key) {
				if (key in value) checkSchema(props[key], value[key], path + "." + key, errors);
			});
		}
		if (Array.isArray(schema.anyOf) && !schema.anyOf.some(function (sub) { return checkSchema(sub, value, path, []).length === 0; })) {
			errors.push(path + ": value matches no schema in anyOf");
		}
		return errors;
	}

	// ---------- theme ----------
	function themeTokens() {
		var tokens = {};
		var base = state.mode === "dark" ? H.themes.dark : H.themes.light;
		Object.keys(base).forEach(function (k) { tokens[k] = base[k]; });
		var overrides = state.overrides[state.mode];
		Object.keys(overrides).forEach(function (k) { tokens[k] = overrides[k]; });
		return tokens;
	}

	function currentTheme() { return { mode: state.mode, tokens: themeTokens() }; }

	function sendTheme() { send("theme:change", currentTheme()); }

	function setMode(mode) {
		state.mode = mode;
		document.body.classList.toggle("dark", mode === "dark");
		$("theme-toggle").textContent = mode;
		renderTokenEditor();
		if (state.session) sendTheme();
	}

	function renderTokenEditor() {
		var box = $("token-editor");
		box.textContent = "";
		var tokens = themeTokens();
		Object.keys(tokens).forEach(function (name) {
			var row = make("div", "token-row");
			row.appendChild(make("span", "token-name", name));
			var input = document.createElement("input");
			input.type = "text";
			input.value = tokens[name];
			input.addEventListener("change", function () {
				state.overrides[state.mode][name] = input.value;
				sendTheme();
			});
			row.appendChild(input);
			box.appendChild(row);
		});
	}

	// ---------- flw/1 host ----------
	function send(type, payload) {
		var s = state.session;
		if (!s || !s.iframe.contentWindow) return;
		trace("out", type, payload);
		s.iframe.contentWindow.postMessage({
			protocol: H.protocol,
			nonce: s.nonce,
			instanceId: s.instanceId,
			type: type,
			payload: payload
		}, "*");
	}

	function currentProps() {
		var props = {};
		Object.keys(state.props).forEach(function (key) {
			if (state.props[key] !== undefined) props[key] = state.props[key];
		});
		return props;
	}

	function sendInit() {
		send("init", {
			props: currentProps(),
			theme: currentTheme(),
			locale: navigator.language || "en",
			instanceId: state.session.instanceId,
			capabilities: state.preview ? { preview: true } : {}
		});
	}

	function setStatus(text, cls) {
		var status = $("status");
		status.textContent = text;
		status.className = cls || "";
	}

	function unmount() {
		if (state.session) {
			state.session.iframe.remove();
			state.session = null;
		}
	}

	function mount() {
		unmount();
		var widget = state.selected;
		if (!widget) return;
		var iframe = document.createElement("iframe");
		iframe.setAttribute("sandbox", "allow-scripts");
		iframe.src = widget.entryUrl;
		var defaultHeight = (state.contract && state.contract.sizing && state.contract.sizing.defaultHeight) || 320;
		iframe.style.height = defaultHeight + "px";
		state.session = {
			nonce: uid(),
			instanceId: "harness-" + widget.id + "-" + Date.now().toString(36),
			iframe: iframe,
			ready: false
		};
		$("height-display").textContent = "height " + defaultHeight + "px (default)";
		$("stage").appendChild(iframe);
		iframe.addEventListener("load", function () {
			setTimeout(function () {
				if (state.session && state.session.iframe === iframe && !state.session.ready) sendInit();
			}, 50);
		});
		setStatus("connecting to " + widget.entryUrl + " \\u2026");
	}

	window.addEventListener("message", function (event) {
		var s = state.session;
		if (!s || event.source !== s.iframe.contentWindow) return;
		var data = event.data;
		if (!data || data.protocol !== H.protocol || typeof data.type !== "string") return;
		trace("in", data.type, data.payload);
		if (data.type === "hello") { sendInit(); return; }
		if (data.nonce !== s.nonce) { traceNote("dropped '" + data.type + "': nonce mismatch"); return; }
		if (data.type === "ready") {
			s.ready = true;
			setStatus("ready \\u00b7 contract v" + (data.payload ? data.payload.contractVersion : "?") +
				(state.preview ? " \\u00b7 preview" : ""), "ok");
		} else if (data.type === "event") {
			onEvent(data.payload || {});
		} else if (data.type === "query:result") {
			onQueryResult(data.payload || {});
		} else if (data.type === "resize") {
			onResize(data.payload || {});
		} else if (data.type === "value:changed") {
			logEntry("value:changed", (data.payload || {}).values, [], true);
		}
	});

	function onEvent(payload) {
		var name = typeof payload.name === "string" ? payload.name : "(unnamed)";
		var errors = [];
		var events = (state.contract && state.contract.events) || {};
		if (!Object.prototype.hasOwnProperty.call(events, name)) {
			errors.push("not declared in the contract");
		} else if (events[name].payloadSchema) {
			checkSchema(events[name].payloadSchema, payload.payload, "$", errors);
		} else if (payload.payload !== undefined && payload.payload !== null) {
			errors.push("contract declares no payload, but one was sent");
		}
		logEntry(name, payload.payload, errors, false);
	}

	function logEntry(name, payload, errors, meta) {
		var entry = make("div", "log-entry" + (errors.length ? " invalid" : "") + (meta ? " meta" : ""));
		var head = make("div", "log-head");
		head.appendChild(make("span", "log-time", timestamp()));
		head.appendChild(make("span", "log-name", name));
		if (errors.length) head.appendChild(make("span", "log-flag", "invalid"));
		entry.appendChild(head);
		entry.appendChild(make("pre", "log-payload", shortJson(payload)));
		errors.forEach(function (err) { entry.appendChild(make("div", "log-error", err)); });
		appendCapped($("event-log"), entry, LOG_CAP);
	}

	function onResize(payload) {
		var s = state.session;
		if (!s) return;
		var reported = typeof payload.height === "number" ? payload.height : 0;
		var maxHeight = (state.contract && state.contract.sizing && state.contract.sizing.maxHeight) || 4000;
		var applied = Math.max(40, Math.min(Math.ceil(reported), maxHeight));
		s.iframe.style.height = applied + "px";
		$("height-display").textContent = "height " + Math.round(reported) + "px" +
			(applied !== Math.ceil(reported) ? " (clamped to " + applied + "px)" : "");
	}

	// ---------- props panel ----------
	function seedProps() {
		var props = {};
		state.formModel.forEach(function (field) {
			if (field.default !== undefined) props[field.key] = field.default;
		});
		state.props = props;
	}

	function sendProp(key, value) {
		state.props[key] = value;
		var patch = {};
		patch[key] = value;
		send("props:update", { props: patch });
	}

	function renderPropsPanel() {
		var panel = $("props-panel");
		panel.textContent = "";
		if (!state.formModel.length) {
			panel.appendChild(make("div", "empty", "No inputs declared in the contract."));
			return;
		}
		state.formModel.forEach(function (field) { panel.appendChild(fieldRow(field)); });
	}

	function fieldRow(field) {
		var row = make("div", "field");
		row.appendChild(make("label", "field-label", field.key + (field.optional ? " (optional)" : "")));
		if (field.description) row.appendChild(make("div", "field-desc", field.description));
		var control = field.control;
		var value = state.props[field.key];
		if (control.kind === "checkbox") {
			var checkbox = document.createElement("input");
			checkbox.type = "checkbox";
			checkbox.checked = value === true;
			checkbox.addEventListener("change", function () { sendProp(field.key, checkbox.checked); });
			row.appendChild(checkbox);
		} else if (control.kind === "select") {
			var select = document.createElement("select");
			control.choices.forEach(function (choice) {
				var option = document.createElement("option");
				option.value = choice;
				option.textContent = choice;
				select.appendChild(option);
			});
			if (typeof value === "string") select.value = value;
			select.addEventListener("change", function () { sendProp(field.key, select.value); });
			row.appendChild(select);
		} else if (control.kind === "number") {
			var num = document.createElement("input");
			num.type = "number";
			if (control.min !== undefined) num.min = String(control.min);
			if (control.max !== undefined) num.max = String(control.max);
			num.step = control.integer ? "1" : "any";
			if (typeof value === "number") num.value = String(value);
			var numError = make("div", "field-error");
			num.addEventListener("change", function () {
				var parsed = control.integer ? parseInt(num.value, 10) : Number(num.value);
				if (num.value === "" || isNaN(parsed)) { numError.textContent = "not a number"; return; }
				if (control.min !== undefined && parsed < control.min) { numError.textContent = "minimum is " + control.min; return; }
				if (control.max !== undefined && parsed > control.max) { numError.textContent = "maximum is " + control.max; return; }
				numError.textContent = "";
				sendProp(field.key, parsed);
			});
			row.appendChild(num);
			row.appendChild(numError);
		} else if (control.kind === "json") {
			var area = document.createElement("textarea");
			area.rows = 4;
			area.spellcheck = false;
			area.value = value === undefined ? "" : JSON.stringify(value, null, 2);
			var jsonError = make("div", "field-error");
			area.addEventListener("input", function () {
				if (area.value.trim() === "") { area.classList.remove("bad"); jsonError.textContent = ""; return; }
				var parsed;
				try { parsed = JSON.parse(area.value); } catch (e) {
					area.classList.add("bad");
					jsonError.textContent = "invalid JSON: " + e.message;
					return;
				}
				area.classList.remove("bad");
				var errors = control.schema ? checkSchema(control.schema, parsed, "$", []) : [];
				jsonError.textContent = errors.join("; ");
				sendProp(field.key, parsed);
			});
			row.appendChild(area);
			row.appendChild(jsonError);
		} else {
			var text = document.createElement("input");
			text.type = "text";
			if (typeof value === "string") text.value = value;
			text.addEventListener("input", function () { sendProp(field.key, text.value); });
			row.appendChild(text);
		}
		return row;
	}

	// ---------- fixtures ----------
	function renderFixtures() {
		var box = $("fixtures-panel");
		box.textContent = "";
		var names = Object.keys(state.fixtures);
		if (!names.length) {
			box.appendChild(make("div", "empty", "No dev.fixtures declared in widget.config.ts."));
			return;
		}
		names.forEach(function (name) {
			var button = make("button", "fixture-btn", name);
			button.type = "button";
			button.addEventListener("click", function () {
				var patch = state.fixtures[name];
				Object.keys(patch).forEach(function (key) { state.props[key] = patch[key]; });
				renderPropsPanel();
				send("props:update", { props: patch });
			});
			box.appendChild(button);
		});
	}

	// ---------- query invoker ----------
	function renderQueryPanel() {
		var select = $("query-select");
		select.textContent = "";
		var queries = (state.contract && state.contract.queries) || {};
		var names = Object.keys(queries);
		$("query-invoke").disabled = names.length === 0;
		if (!names.length) {
			var placeholder = document.createElement("option");
			placeholder.textContent = "(no queries declared)";
			select.appendChild(placeholder);
			return;
		}
		names.forEach(function (name) {
			var option = document.createElement("option");
			option.value = name;
			option.textContent = name;
			select.appendChild(option);
		});
	}

	function invokeQuery() {
		var name = $("query-select").value;
		var queries = (state.contract && state.contract.queries) || {};
		if (!Object.prototype.hasOwnProperty.call(queries, name)) return;
		var result = $("query-result");
		var argsText = $("query-args").value.trim();
		var args = null;
		if (argsText !== "") {
			try { args = JSON.parse(argsText); } catch (e) {
				result.textContent = "invalid args JSON: " + e.message;
				result.className = "err";
				return;
			}
		}
		var queryId = uid();
		state.pending[queryId] = {
			name: name,
			timer: setTimeout(function () {
				if (state.pending[queryId]) {
					delete state.pending[queryId];
					result.textContent = "query '" + name + "' timed out after 10s";
					result.className = "err";
				}
			}, 10000)
		};
		result.textContent = "waiting \\u2026";
		result.className = "";
		send("query", { queryId: queryId, name: name, args: args });
	}

	function onQueryResult(payload) {
		var pending = state.pending[payload.queryId];
		if (!pending) { traceNote("query:result for unknown queryId " + shortJson(payload.queryId)); return; }
		clearTimeout(pending.timer);
		delete state.pending[payload.queryId];
		var box = $("query-result");
		if (!payload.ok) {
			box.textContent = pending.name + " failed: " + payload.error;
			box.className = "err";
			return;
		}
		var text;
		try { text = JSON.stringify(payload.value, null, 2); } catch (e) { text = String(payload.value); }
		box.textContent = pending.name + " \\u2192 " + (text === undefined ? "undefined" : text);
		box.className = "ok";
		var spec = ((state.contract && state.contract.queries) || {})[pending.name];
		if (spec && spec.resultSchema) {
			var errors = checkSchema(spec.resultSchema, payload.value, "$", []);
			if (errors.length) {
				box.className = "warn";
				box.textContent += "\\n(schema mismatch: " + errors.join("; ") + ")";
			}
		}
	}

	// ---------- selection & boot ----------
	function renderSidebar() {
		var nav = $("widget-list");
		nav.textContent = "";
		var byGroup = {};
		state.widgets.forEach(function (widget) {
			(byGroup[widget.group] = byGroup[widget.group] || []).push(widget);
		});
		Object.keys(byGroup).sort().forEach(function (group) {
			nav.appendChild(make("div", "group-name", group));
			byGroup[group].forEach(function (widget) {
				var button = make("button", "widget-item", widget.id);
				button.type = "button";
				button.dataset.key = widget.group + "/" + widget.id;
				if (state.selected && state.selected.group === widget.group && state.selected.id === widget.id) {
					button.classList.add("active");
				}
				button.addEventListener("click", function () { selectWidget(widget); });
				nav.appendChild(button);
			});
		});
	}

	function selectWidget(widget) {
		state.selected = widget;
		renderSidebar();
		$("widget-title").textContent = widget.group + "/" + widget.id;
		$("event-log").textContent = "";
		$("query-result").textContent = "";
		$("query-result").className = "";
		loadContract();
	}

	function loadContract() {
		var widget = state.selected;
		if (!widget) return;
		setStatus("loading contract \\u2026");
		fetch("/api/contract/" + encodeURIComponent(widget.group) + "/" + encodeURIComponent(widget.id))
			.then(function (response) {
				return response.json().then(function (body) { return { ok: response.ok, body: body }; });
			})
			.then(function (result) {
				if (state.selected !== widget) return;
				if (!result.ok) {
					state.contract = null;
					state.formModel = [];
					state.fixtures = {};
					setStatus("contract error: " + (result.body && result.body.error), "err");
				} else {
					state.contract = result.body.contract;
					state.formModel = result.body.formModel || [];
					state.fixtures = result.body.fixtures || {};
					(result.body.warnings || []).forEach(function (warning) {
						traceNote("contract warning: " + warning);
					});
				}
				seedProps();
				renderPropsPanel();
				renderFixtures();
				renderQueryPanel();
				mount();
			})
			.catch(function (error) {
				if (state.selected !== widget) return;
				setStatus("contract fetch failed: " + error, "err");
			});
	}

	function refreshWidgets(autoselect) {
		return fetch("/api/widgets")
			.then(function (response) { return response.json(); })
			.then(function (body) {
				if (body && Array.isArray(body.widgets) && body.widgets.length) {
					state.widgets = body.widgets;
					if (state.selected) {
						var again = state.widgets.filter(function (widget) {
							return widget.group === state.selected.group && widget.id === state.selected.id;
						})[0];
						if (again) state.selected = again;
					}
					renderSidebar();
				}
			})
			.catch(function () {})
			.then(function () {
				if (autoselect && !state.selected && state.widgets.length) selectWidget(state.widgets[0]);
			});
	}

	function boot() {
		renderSidebar();
		renderTokenEditor();
		$("width-select").addEventListener("change", function () {
			var value = $("width-select").value;
			$("stage").style.width = value === "full" ? "100%" : value + "px";
		});
		$("theme-toggle").addEventListener("click", function () {
			setMode(state.mode === "dark" ? "light" : "dark");
		});
		$("preview-toggle").addEventListener("change", function (event) {
			state.preview = event.target.checked;
			mount();
		});
		$("reload-btn").addEventListener("click", function () { loadContract(); });
		$("query-invoke").addEventListener("click", invokeQuery);
		$("clear-log").addEventListener("click", function (event) {
			event.preventDefault();
			event.stopPropagation();
			$("event-log").textContent = "";
		});
		$("clear-trace").addEventListener("click", function (event) {
			event.preventDefault();
			event.stopPropagation();
			$("trace").textContent = "";
		});
		if (window.matchMedia && window.matchMedia("(prefers-color-scheme: dark)").matches) {
			setMode("dark");
		}
		refreshWidgets(true);
	}

	boot();
})();
`;

/**
 * Generate the self-contained mock-host harness page (inline HTML/CSS/JS,
 * no frameworks, no external assets). Pure function of the widget list.
 */
export function harnessHtml(widgets: HarnessWidget[]): string {
	const data: HarnessData = {
		protocol: FLW_PROTOCOL,
		widgets,
		themes: { light: DEFAULT_LIGHT_TOKENS, dark: DEFAULT_DARK_TOKENS },
	};
	return `<!doctype html>
<html lang="en">
	<head>
		<meta charset="utf-8" />
		<meta name="viewport" content="width=device-width, initial-scale=1" />
		<title>Flow-Like Widget Harness</title>
		<style>${HARNESS_CSS}</style>
		<script>window.__HARNESS__ = ${embedJson(data)};</script>
	</head>
	<body>
		<div id="app">
			<aside id="sidebar">
				<h1>Flow-Like <span>widget harness</span></h1>
				<nav id="widget-list">
${sidebarMarkup(widgets)}
				</nav>
			</aside>
			<main id="main">
				<div id="toolbar">
					<span id="widget-title">select a widget</span>
					<span id="status"></span>
					<span class="spacer"></span>
					<label class="control">width
						<select id="width-select">
							<option value="full" selected>full</option>
							<option value="360">360</option>
							<option value="480">480</option>
							<option value="768">768</option>
							<option value="1024">1024</option>
						</select>
					</label>
					<span id="height-display" class="control"></span>
					<label class="control"><input type="checkbox" id="preview-toggle" /> preview</label>
					<button id="theme-toggle" type="button" title="Toggle light/dark theme">light</button>
					<button id="reload-btn" type="button" title="Re-extract the contract and remount">reload</button>
				</div>
				<div id="content">
					<div id="stage-wrap"><div id="stage"></div></div>
					<div id="panels">
						<details open>
							<summary>Props</summary>
							<div id="props-panel"></div>
						</details>
						<details open>
							<summary>Fixtures</summary>
							<div id="fixtures-panel"></div>
						</details>
						<details open>
							<summary>Events <button id="clear-log" type="button">clear</button></summary>
							<div id="event-log"></div>
						</details>
						<details open>
							<summary>Query</summary>
							<div id="query-panel">
								<select id="query-select"></select>
								<textarea id="query-args" rows="3" placeholder="args (JSON, empty = null)"></textarea>
								<button id="query-invoke" type="button">invoke</button>
								<pre id="query-result"></pre>
							</div>
						</details>
						<details>
							<summary>Theme tokens</summary>
							<div id="token-editor"></div>
						</details>
						<details>
							<summary>Protocol trace <button id="clear-trace" type="button">clear</button></summary>
							<div id="trace"></div>
						</details>
					</div>
				</div>
			</main>
		</div>
		<script>${HARNESS_JS}</script>
	</body>
</html>
`;
}
