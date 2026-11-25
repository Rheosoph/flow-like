import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import {
	existsSync,
	mkdirSync,
	mkdtempSync,
	readFileSync,
	rmSync,
	statSync,
	utimesSync,
	writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { pathToFileURL } from "node:url";
import { writeSourceFile } from "./build-cache.mjs";
import { initializeWasmStartup, prepareWasmStartup } from "./wasm-startup.mjs";

const ids = ["wordcraft", "pdfcraft", "photocraft", "vectorcraft"];

function temporary(t) {
	const directory = mkdtempSync(join(tmpdir(), "craft-wasm-startup-"));
	t.after(() => rmSync(directory, { recursive: true, force: true }));
	return directory;
}

function checkout(t, build = "") {
	const source = temporary(t);
	const git = (...args) =>
		execFileSync("git", args, {
			cwd: source,
			encoding: "utf8",
			stdio: ["ignore", "pipe", "pipe"],
		});
	git("init", "-q");
	const commit = () => {
		git("add", ".");
		git(
			"-c",
			"user.name=Test",
			"-c",
			"user.email=test@example.invalid",
			"-c",
			"core.hooksPath=/dev/null",
			"commit",
			"--no-gpg-sign",
			"-qm",
			"Fixture",
		);
	};
	for (const id of ids) {
		const directory = join(source, "apps", `${id}-web`);
		mkdirSync(directory, { recursive: true });
		writeFileSync(
			join(directory, "Cargo.toml"),
			`[package]\nname = "${id}-web"\nversion = "0.1.0"\n${build}\n[dependencies]\n`,
		);
	}
	commit();
	return {
		source,
		commit,
		original: (path) => git("show", `HEAD:${path}`),
		write: (path, text) => writeSourceFile(source, path, text),
	};
}

const loader = `let wasmModule, wasmInstance, wasm;
function __wbg_finalize_init(instance, module) {
    wasmInstance = instance;
    wasm = instance.exports;
    wasmModule = module;
    wasm.__wbindgen_start();
    return wasm;
}
export function initSync(module, imports) {
    if (wasm !== undefined) return wasm;
    return __wbg_finalize_init(new WebAssembly.Instance(module, imports), module);
}
export default async function init(bytes, imports) {
    if (wasm !== undefined) return wasm;
    const { instance, module } = await WebAssembly.instantiate(bytes, imports);
    return __wbg_finalize_init(instance, module);
}
`;

function uint(value) {
	const bytes = [];
	let remaining = value;
	while (remaining > 127) {
		bytes.push((remaining & 127) | 128);
		remaining >>>= 7;
	}
	return [...bytes, remaining];
}
const text = (value) => [
	...uint(Buffer.byteLength(value)),
	...Buffer.from(value),
];
const section = (id, bytes) => [id, ...uint(bytes.length), ...bytes];

function wasm({
	exportConstructors = true,
	kind = 0,
	runnerIndex = 2,
	runner = [0, 0x10, 0, 0x0b],
} = {}) {
	return Buffer.from([
		0,
		97,
		115,
		109,
		1,
		0,
		0,
		0,
		...section(1, [1, 0x60, 0, 0]),
		...section(2, [
			2,
			...text("env"),
			...text("ctors"),
			0,
			0,
			...text("env"),
			...text("start"),
			0,
			0,
		]),
		...section(3, [1, 0]),
		...section(7, [
			exportConstructors ? 2 : 1,
			...(exportConstructors
				? [...text("__wasm_call_ctors"), kind, ...uint(runnerIndex)]
				: []),
			...text("__wbindgen_start"),
			0,
			1,
		]),
		...section(10, [1, ...uint(runner.length), ...runner]),
	]);
}

function packaged(t, { id = "photocraft", js = loader, binary = wasm() } = {}) {
	const output = temporary(t);
	const moduleName = id === "wordcraft" ? "wordcraft" : "craft";
	writeFileSync(join(output, "package.json"), '{"type":"module"}');
	writeFileSync(join(output, `${moduleName}.js`), js);
	writeFileSync(join(output, `${moduleName}_bg.wasm`), binary);
	return { id, output, moduleName };
}

test("preparation exports constructors only from the four affected web packages", (t) => {
	const fixture = checkout(t);
	for (const id of ids) {
		assert.equal(prepareWasmStartup({ ...fixture, id }), true);
		const path = join(fixture.source, `apps/${id}-web/build.rs`);
		const script = readFileSync(path, "utf8");
		assert.match(script, /CARGO_CFG_TARGET_ARCH.*Ok\("wasm32"\)/);
		assert.match(script, /cargo:rustc-link-arg=--export=__wasm_call_ctors/);
		assert.doesNotMatch(script, /unsafe/);
		utimesSync(path, new Date(0), new Date(0));
		prepareWasmStartup({ ...fixture, id });
		assert.equal(
			statSync(path).mtimeMs,
			0,
			"Warm preparation keeps the owned file unchanged.",
		);
	}
});

test("unaffected editors do not inspect or change source or output", () => {
	for (const id of [
		"cadcraft",
		"deckcraft",
		"designcraft",
		"filmcraft",
		"gridcraft",
		"soundcraft",
		"solidcraft",
	]) {
		assert.equal(prepareWasmStartup({ id }), false);
		assert.equal(initializeWasmStartup({ id }), false);
	}
});

test("preparation refuses upstream build scripts even when they match its owned bytes", (t) => {
	const fixture = checkout(t);
	prepareWasmStartup({ ...fixture, id: "photocraft" });
	fixture.commit();
	assert.throws(
		() => prepareWasmStartup({ ...fixture, id: "photocraft" }),
		/upstream has a build script/,
	);
});

test("preparation refuses unrecognized local build scripts", (t) => {
	const fixture = checkout(t);
	const path = join(fixture.source, "apps/photocraft-web/build.rs");
	writeFileSync(path, "fn main() { panic!(); }\n");
	assert.throws(
		() => prepareWasmStartup({ ...fixture, id: "photocraft" }),
		/unrecognized build script/,
	);
	assert.equal(readFileSync(path, "utf8"), "fn main() { panic!(); }\n");
});

for (const build of ["build = false", 'build = "other.rs"']) {
	test(`preparation rejects upstream ${build}`, (t) => {
		const fixture = checkout(t, build);
		assert.throws(
			() => prepareWasmStartup({ ...fixture, id: "photocraft" }),
			/custom or disabled/,
		);
		assert.equal(
			existsSync(join(fixture.source, "apps/photocraft-web/build.rs")),
			false,
		);
	});
}

test("preparation checks the prepared manifest as well as the upstream snapshot", (t) => {
	const fixture = checkout(t);
	const path = join(fixture.source, "apps/photocraft-web/Cargo.toml");
	writeFileSync(
		path,
		readFileSync(path, "utf8").replace("[package]", "[package]\nbuild = false"),
	);
	assert.throws(
		() => prepareWasmStartup({ ...fixture, id: "photocraft" }),
		/custom or disabled/,
	);
});

for (const mode of ["sync", "async"]) {
	test(`${mode} initialization runs constructors before startup once per instance`, async (t) => {
		const fixture = packaged(t, {
			id: mode === "sync" ? "wordcraft" : "photocraft",
		});
		assert.equal(initializeWasmStartup(fixture), true);
		const module = await import(
			pathToFileURL(join(fixture.output, `${fixture.moduleName}.js`))
		);
		const events = [];
		const imports = {
			env: {
				ctors: () => events.push("constructors"),
				start: () => events.push("start"),
			},
		};
		const binary = wasm();
		const compiled = new WebAssembly.Module(binary);
		const instance =
			mode === "sync"
				? module.initSync(compiled, imports)
				: await module.default(binary, imports);
		assert.deepEqual(events, ["constructors", "start"]);
		assert.equal(module.initSync(compiled, imports), instance);
		assert.equal(await module.default(binary, imports), instance);
		assert.deepEqual(events, ["constructors", "start"]);
	});
}

test("a second startup patch is rejected without duplicating initialization", (t) => {
	const fixture = packaged(t);
	initializeWasmStartup(fixture);
	const path = join(fixture.output, "craft.js");
	const before = readFileSync(path, "utf8");
	assert.throws(
		() => initializeWasmStartup(fixture),
		/review the wasm-bindgen startup/,
	);
	assert.equal(readFileSync(path, "utf8"), before);
});

test("startup rejects changed or ambiguous generated loader seams", (t) => {
	for (const js of [
		loader.replace("wasm.__wbindgen_start();", "start();"),
		`${loader}\nwasm.__wbindgen_start();`,
		loader
			.replace(
				"    wasm = instance.exports;",
				"    wasm.__wbindgen_start();\n    wasm = instance.exports;",
			)
			.replace(
				"    wasm.__wbindgen_start();\n    return wasm;",
				"    return wasm;",
			),
		loader.replace(
			"function __wbg_finalize_init(instance, module)",
			"function renamed(instance, module)",
		),
	]) {
		const fixture = packaged(t, { js });
		assert.throws(
			() => initializeWasmStartup(fixture),
			/review the wasm-bindgen startup/,
		);
		assert.equal(readFileSync(join(fixture.output, "craft.js"), "utf8"), js);
	}
});

test("startup requires an intact constructor function export before modifying the loader", (t) => {
	for (const binary of [
		wasm({ exportConstructors: false }),
		wasm({ kind: 2 }),
		wasm({ runnerIndex: 0 }),
		wasm({ runnerIndex: 3 }),
		wasm().subarray(0, -1),
		Buffer.from("not wasm"),
	]) {
		const fixture = packaged(t, { binary });
		assert.throws(
			() => initializeWasmStartup(fixture),
			/must export __wasm_call_ctors as a function/,
		);
		assert.equal(
			readFileSync(join(fixture.output, "craft.js"), "utf8"),
			loader,
		);
	}
});

test("startup rejects empty, indirect, recursive or malformed local constructor runners", (t) => {
	for (const [description, runner] of [
		["empty LLD runner", [0, 0x0b]],
		["nop only", [0, 0x01, 0x0b]],
		["self recursion", [0, 0x10, 2, 0x0b]],
		["out-of-range call", [0, 0x10, 3, 0x0b]],
		["indirect call", [0, 0x11, 0, 0, 0x0b]],
		["unexpected locals", [1, 1, 0x7f, 0x10, 0, 0x0b]],
		["missing end", [0, 0x10, 0]],
		["truncated call index", [0, 0x10, 0x80]],
		["trailing instructions", [0, 0x10, 0, 0x0b, 0x01]],
	]) {
		const fixture = packaged(t, { binary: wasm({ runner }) });
		assert.throws(
			() => initializeWasmStartup(fixture),
			/nonempty local constructor runner/,
			description,
		);
		assert.equal(
			readFileSync(join(fixture.output, "craft.js"), "utf8"),
			loader,
		);
	}
});
