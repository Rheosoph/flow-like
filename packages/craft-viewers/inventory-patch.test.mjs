import assert from "node:assert/strict";
import test from "node:test";
import {
	addInventoryPatch,
	assertInventoryLock,
	patchInventoryLibrary,
	patchInventoryManifest,
	prepareInventory,
} from "./inventory-patch.mjs";

const lock = `version = 4
[[package]]
name = "inventory"
version = "0.3.25"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "6928282826c822ad91bf1c9a1cb90a30ba1c26770749929b4656cd6be829cd7c"
dependencies = ["rustversion"]
`;
const replaceOnce = (text, search, replacement) => {
	assert.equal(
		text.split(search).length,
		2,
		"The reviewed source seam must be unique.",
	);
	return text.replace(search, replacement);
};

test("only the four affected editors prepare an inventory override", () => {
	for (const id of [
		"cadcraft",
		"deckcraft",
		"designcraft",
		"filmcraft",
		"gridcraft",
		"soundcraft",
		"solidcraft",
	])
		assert.deepEqual(prepareInventory({ id }), []);
});

test("preparation rejects changed registry identities before accessing its source or cache", () => {
	assert.doesNotThrow(() => assertInventoryLock(lock));
	for (const changed of [
		lock.replace('version = "0.3.25"', 'version = "0.3.26"'),
		lock.replace(
			'source = "registry+https://github.com/rust-lang/crates.io-index"',
			'source = "git+https://example.invalid/inventory#abc"',
		),
		lock.replace("69282828", "00000000"),
		`${lock}\n${lock}`,
		lock.replace('name = "inventory"', 'name = "another-package"'),
	]) {
		assert.throws(
			() => prepareInventory({ id: "photocraft", original: () => changed }),
			/Review the pinned inventory registry package/,
		);
	}
});

test("the source correction only broadens the stable release condition to beta", () => {
	const source = `before\n#[cfg_attr(target_family = "wasm", attr(any(all(stable, since(1.85)), since(2024-12-18)), link_section = ".init_array"))]\nstatic __CTOR: unsafe extern "C" fn() = __ctor;\nafter\n`;
	const patched = patchInventoryLibrary(source, replaceOnce);
	assert.equal(patched.replace("any(stable, beta)", "stable"), source);
	assert.throws(() => patchInventoryLibrary(patched, replaceOnce));
	assert.throws(() => patchInventoryLibrary(`${source}${source}`, replaceOnce));
});

test("the standalone manifest keeps runtime dependencies and metadata while removing development dependencies", () => {
	const before = `[package]\nname = "inventory"\nversion = "0.3.25"\nrepository = "https://github.com/dtolnay/inventory"\nlicense = "MIT OR Apache-2.0"\n\n[target.'cfg(target_family = "wasm")'.dependencies]\nrustversion = "1.0"\n\n`;
	const dev = '[dev-dependencies]\nrustversion = "1.0"\ntrybuild = "1.0"\n\n';
	const after =
		'[package.metadata.docs.rs]\ntargets = ["x86_64-unknown-linux-gnu"]\n';
	assert.equal(patchInventoryManifest(before + dev + after), before + after);
	assert.throws(
		() =>
			patchInventoryManifest(
				(before + dev).replace('version = "0.3.25"', 'version = "0.3.26"'),
			),
		/version changed/,
	);
	assert.throws(
		() => patchInventoryManifest(before + after),
		/development dependencies changed/,
	);
});

test("adding the crates.io override preserves other Git and registry patches", () => {
	const workspace =
		'[workspace]\nmembers = ["apps/demo"]\n\n[patch."https://example.invalid/shared"]\nshared = { path = "shared" }\n\n[patch.crates-io]\nother = { path = "other" }\n\n[profile.release]\nopt-level = 3\n';
	const dependency = 'inventory = { path = "host-dependencies/inventory" }\n';
	const patched = addInventoryPatch(workspace);
	assert.equal(patched.replace(dependency, ""), workspace);
	assert.equal(addInventoryPatch(patched), patched);
	const without = "[workspace]\nmembers = []\n";
	assert.equal(
		addInventoryPatch(without),
		`${without}\n[patch.crates-io]\n${dependency}`,
	);
});

test("existing inventory patches and ambiguous tables fail without being overwritten", () => {
	for (const workspace of [
		'[patch.crates-io]\ninventory = { git = "https://example.invalid/fork" }\n',
		'[patch."crates-io"]\n"inventory" = { path = "other" }\n',
		'[patch.crates-io.inventory]\npath = "other"\n',
		"[patch.crates-io]\nother = {}\n[patch.'crates-io']\nanother = {}\n",
	])
		assert.throws(
			() => addInventoryPatch(workspace),
			/inventory crates.io patch/,
		);
});
