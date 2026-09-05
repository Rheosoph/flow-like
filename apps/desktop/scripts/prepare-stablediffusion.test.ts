import { afterEach, describe, expect, test } from "bun:test";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import AdmZip from "adm-zip";
import {
	extractRuntime,
	prepare,
	stageWindowsRuntimeDlls,
} from "./prepare-stablediffusion";

const temporaryDirectories: string[] = [];

function temporaryDirectory() {
	const directory = fs.mkdtempSync(
		path.join(os.tmpdir(), "flow-like-sd-runtime-test-"),
	);
	temporaryDirectories.push(directory);
	return directory;
}

function archive(files: Record<string, string>) {
	const zip = new AdmZip();
	for (const [name, contents] of Object.entries(files)) {
		zip.addFile(name, Buffer.from(contents));
	}
	return zip.toBuffer();
}

afterEach(() => {
	for (const directory of temporaryDirectories.splice(0)) {
		fs.rmSync(directory, { recursive: true, force: true });
	}
});

describe("stable-diffusion.cpp runtime archive extraction", () => {
	test("flattens the executable and private libraries into an isolated directory", () => {
		const root = temporaryDirectory();
		const destination = path.join(root, "stablediffusion");
		const llama = path.join(root, "llama");
		fs.mkdirSync(destination);
		fs.mkdirSync(llama);
		fs.writeFileSync(path.join(llama, "libggml.so"), "llama library");

		extractRuntime(
			archive({
				"release/bin/sd-server": "diffusion server",
				"release/lib/libggml.so": "diffusion library",
				"release/lib/libggml.so.0.9.6": "versioned library",
				"release/share/ggml-metal.metal": "metal shader",
				"release/LICENSE.txt": "license",
				"release/bin/sd-cli": "unneeded tool",
				"release/include/stable-diffusion.h": "unneeded header",
			}),
			destination,
			"sd-server",
		);

		expect(fs.readdirSync(destination).sort()).toEqual([
			"LICENSE.txt",
			"ggml-metal.metal",
			"libggml.so",
			"libggml.so.0.9.6",
			"sd-server",
		]);
		expect(fs.readFileSync(path.join(destination, "libggml.so"), "utf8")).toBe(
			"diffusion library",
		);
		expect(fs.readFileSync(path.join(llama, "libggml.so"), "utf8")).toBe(
			"llama library",
		);
		if (process.platform !== "win32") {
			expect(
				fs.statSync(path.join(destination, "sd-server")).mode & 0o777,
			).toBe(0o755);
		}
	});

	test("rejects duplicate flattened library names", () => {
		const destination = temporaryDirectory();
		expect(() =>
			extractRuntime(
				archive({
					"release/bin/sd-server": "server",
					"release/lib/libggml.so": "first library",
					"release/other/libggml.so": "second library",
				}),
				destination,
				"sd-server",
			),
		).toThrow("Duplicate runtime file in archive: libggml.so");
	});

	test("rejects library names that collide on Windows", () => {
		expect(() =>
			extractRuntime(
				archive({
					"bin/sd-server.exe": "server",
					"lib/GGML.dll": "first",
					"other/ggml.dll": "second",
				}),
				temporaryDirectory(),
				"sd-server.exe",
			),
		).toThrow("Duplicate runtime file in archive");
	});

	test("rejects duplicate executable basenames", () => {
		expect(() =>
			extractRuntime(
				archive({ "bin/sd-server": "first", "other/sd-server": "second" }),
				temporaryDirectory(),
				"sd-server",
			),
		).toThrow("Duplicate runtime file in archive: sd-server");
	});

	test("rejects archives that contain libraries without the requested server", () => {
		expect(() =>
			extractRuntime(
				archive({ "lib/libggml.so": "library", "bin/sd-cli": "tool" }),
				temporaryDirectory(),
				"sd-server",
			),
		).toThrow("Release archive does not contain sd-server");
	});

	test("extracts Windows executable and DLL names from nested archive paths", () => {
		const destination = temporaryDirectory();
		extractRuntime(
			archive({
				"release/bin/sd-server.exe": "windows server",
				"release/bin/ggml.dll": "private DLL",
				"release/bin/sd-cli.exe": "unneeded tool",
			}),
			destination,
			"sd-server.exe",
		);
		expect(fs.readdirSync(destination).sort()).toEqual([
			"ggml.dll",
			"sd-server.exe",
		]);
		expect(fs.readFileSync(path.join(destination, "ggml.dll"), "utf8")).toBe(
			"private DLL",
		);
	});
});

test("unsupported platforms fail before downloading or modifying a runtime", async () => {
	await expect(prepare("linux-arm64")).rejects.toThrow(
		"Unsupported sd-server platform: linux-arm64",
	);
});

describe("private Windows runtime dependencies", () => {
	const dlls = [
		"msvcp140.dll",
		"msvcp140_codecvt_ids.dll",
		"vcomp140.dll",
		"vcruntime140.dll",
		"vcruntime140_1.dll",
	];
	test("stages every DLL imported by the server and its CPU backend", () => {
		const source = temporaryDirectory();
		const destination = temporaryDirectory();
		for (const name of dlls)
			fs.writeFileSync(
				path.join(source, `${name}-x86_64-pc-windows-msvc`),
				name,
			);
		stageWindowsRuntimeDlls(destination, source);
		expect(fs.readdirSync(destination).sort()).toEqual([...dlls].sort());
		for (const name of dlls)
			expect(fs.readFileSync(path.join(destination, name), "utf8")).toBe(name);
	});
	test("rejects a missing OpenMP runtime before copying any DLLs", () => {
		const source = temporaryDirectory();
		const destination = temporaryDirectory();
		for (const name of dlls.filter((name) => name !== "vcomp140.dll"))
			fs.writeFileSync(
				path.join(source, `${name}-x86_64-pc-windows-msvc`),
				name,
			);
		expect(() => stageWindowsRuntimeDlls(destination, source)).toThrow(
			"Missing prepared vcomp140.dll",
		);
		expect(fs.readdirSync(destination)).toEqual([]);
	});
});
