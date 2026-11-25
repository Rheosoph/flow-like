import { execFileSync } from "node:child_process";
import { lstatSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const editors = new Set(["wordcraft", "pdfcraft", "photocraft", "vectorcraft"]);
const buildScript = `fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() == Ok("wasm32") {
        println!("cargo:rustc-link-arg=--export=__wasm_call_ctors");
    }
}
`;

function checkManifest(text, id) {
	const packageSection = text.match(
		/^\[package\][ \t]*(?:#.*)?\r?\n([\s\S]*?)(?=^\[|(?![\s\S]))/m,
	)?.[1];
	const name = packageSection?.match(
		/^[ \t]*name[ \t]*=[ \t]*["']([^"']+)["'][ \t]*(?:#.*)?$/m,
	)?.[1];
	if (name !== `${id}-web`)
		throw new Error(
			`${id}: review the web package before adding WASM startup.`,
		);
	const build = packageSection.match(
		/^[ \t]*build[ \t]*=[ \t]*([^\r\n]+)/m,
	)?.[1];
	if (build && !/^["']build\.rs["'][ \t]*(?:#.*)?$/.test(build))
		throw new Error(
			`${id}: the web package has a custom or disabled build script.`,
		);
}

export function prepareWasmStartup({ id, source, write, original }) {
	if (!editors.has(id)) return false;
	const packagePath = `apps/${id}-web`;
	const manifestPath = `${packagePath}/Cargo.toml`;
	checkManifest(original(manifestPath), id);
	checkManifest(readFileSync(join(source, manifestPath), "utf8"), id);
	const path = `${packagePath}/build.rs`;
	const upstream = execFileSync(
		"git",
		["ls-tree", "--name-only", "HEAD", "--", path],
		{ cwd: source, encoding: "utf8" },
	).trim();
	if (upstream)
		throw new Error(
			`${id}: upstream has a build script; review its WASM startup.`,
		);
	try {
		const destination = join(source, path);
		if (
			!lstatSync(destination).isFile() ||
			readFileSync(destination, "utf8") !== buildScript
		)
			throw new Error(
				`${id}: refusing to replace an unrecognized build script.`,
			);
	} catch (error) {
		if (error.code !== "ENOENT") throw error;
	}
	write(path, buildScript);
	return true;
}

function requireConstructorExport(bytes, id) {
	const fail = () => {
		throw new Error(
			`${id}: WASM must export __wasm_call_ctors as a function with a nonempty local constructor runner.`,
		);
	};
	if (!bytes.subarray(0, 8).equals(Buffer.from([0, 97, 115, 109, 1, 0, 0, 0])))
		fail();
	let offset = 8;
	let limit = bytes.length;
	const uint = () => {
		let value = 0;
		for (let shift = 0; shift <= 28; shift += 7) {
			if (offset >= limit) fail();
			const byte = bytes[offset++];
			if (shift === 28 && byte > 15) fail();
			value += (byte & 127) * 2 ** shift;
			if (byte < 128) return value;
		}
		fail();
	};
	const name = () => {
		const length = uint();
		if (offset + length > limit) fail();
		const value = bytes.toString("utf8", offset, offset + length);
		offset += length;
		return value;
	};
	let importedFunctions = 0;
	let definedFunctions;
	let constructorIndex;
	let code;
	const seen = new Set();
	while (offset < bytes.length) {
		limit = bytes.length;
		const section = bytes[offset++];
		const size = uint();
		const end = offset + size;
		if (end > bytes.length) fail();
		limit = end;
		if ([2, 3, 7, 10].includes(section)) {
			if (seen.has(section)) fail();
			seen.add(section);
		}
		if (section === 2) {
			const count = uint();
			for (let index = 0; index < count; index++) {
				name();
				name();
				// These wasm-bindgen browser modules import functions only.
				if (bytes[offset++] !== 0) fail();
				uint();
				importedFunctions++;
			}
		} else if (section === 3) {
			definedFunctions = uint();
			for (let index = 0; index < definedFunctions; index++) uint();
		} else if (section === 7) {
			const count = uint();
			for (let index = 0; index < count; index++) {
				const exported = name();
				const kind = bytes[offset++];
				const functionIndex = uint();
				if (exported === "__wasm_call_ctors") {
					if (constructorIndex !== undefined || kind !== 0) fail();
					constructorIndex = functionIndex;
				}
			}
		} else if (section === 10) {
			code = { start: offset, end };
		}
		if ([2, 3, 7].includes(section) && offset !== end) fail();
		offset = end;
	}
	const totalFunctions = importedFunctions + definedFunctions;
	if (
		constructorIndex === undefined ||
		!Number.isSafeInteger(totalFunctions) ||
		constructorIndex < importedFunctions ||
		constructorIndex >= totalFunctions ||
		!code
	)
		fail();
	offset = code.start;
	limit = code.end;
	if (uint() !== definedFunctions) fail();
	let checked = false;
	for (let index = 0; index < definedFunctions; index++) {
		const size = uint();
		const end = offset + size;
		if (end > code.end) fail();
		if (index + importedFunctions === constructorIndex) {
			limit = end;
			// LLD's runner has no locals and directly calls each retained constructor.
			if (uint() !== 0) fail();
			let calls = 0;
			while (offset < end) {
				const opcode = bytes[offset++];
				if (opcode === 0x0b) {
					if (!calls || offset !== end) fail();
					checked = true;
					break;
				}
				if (opcode !== 0x10) fail();
				const target = uint();
				if (target === constructorIndex || target >= totalFunctions) fail();
				calls++;
			}
			limit = code.end;
		}
		offset = end;
	}
	if (!checked || offset !== code.end) fail();
}

export function initializeWasmStartup({ id, output, moduleName }) {
	if (!editors.has(id)) return false;
	if (moduleName !== (id === "wordcraft" ? "wordcraft" : "craft"))
		throw new Error(`${id}: unexpected WASM module name.`);
	const path = join(output, `${moduleName}.js`);
	const text = readFileSync(path, "utf8");
	const start = "    wasm.__wbindgen_start();";
	const finalize = "function __wbg_finalize_init(instance, module) {";
	const body = text.match(
		/function __wbg_finalize_init\(instance, module\) \{([\s\S]*?)^\}/m,
	)?.[1];
	if (
		text.includes("__wasm_call_ctors") ||
		text.match(/\bwasm\.__wbindgen_start\s*\(/g)?.length !== 1 ||
		text.match(/function\s+__wbg_finalize_init\s*\(/g)?.length !== 1 ||
		text.split(finalize).length !== 2 ||
		text.split(start).length !== 2 ||
		!body?.includes(start) ||
		body.split("    wasm = instance.exports;").length !== 2 ||
		body.indexOf("    wasm = instance.exports;") > body.indexOf(start)
	)
		throw new Error(
			`${id}: review the wasm-bindgen startup before initializing constructors.`,
		);
	requireConstructorExport(
		readFileSync(join(output, `${moduleName}_bg.wasm`)),
		id,
	);
	// Inventory registers linked command sets before the app installs its UI runtime.
	writeFileSync(
		path,
		text.replace(start, `    wasm.__wasm_call_ctors();\n${start}`),
	);
	return true;
}
