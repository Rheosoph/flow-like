import { execFileSync } from "node:child_process";
import { cpSync, mkdirSync, readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join, resolve } from "node:path";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const debug = process.argv.includes("--debug");
const target = process.env.CARGO_TARGET_DIR || join(root, "target/device-crypto-web");
const tools = join(root, "target/device-crypto-tools");
const lock = readFileSync(join(root, "Cargo.lock"), "utf8");
const version = lock.match(/\[\[package\]\]\s+name = "wasm-bindgen"\s+version = "([^"]+)"/)?.[1];
if (!version) throw new Error("Cargo.lock does not pin wasm-bindgen");
const executable = process.env.WASM_BINDGEN || join(tools, "bin", process.platform === "win32" ? "wasm-bindgen.exe" : "wasm-bindgen");
const run = (command, args) => execFileSync(command, args, { cwd: root, stdio: "inherit", env: { ...process.env, CARGO_TARGET_DIR: target } });
let installed = "";
try { installed = execFileSync(executable, ["--version"], { encoding: "utf8" }).trim(); } catch {}
if (installed !== `wasm-bindgen ${version}`) {
  run("cargo", ["install", "wasm-bindgen-cli", "--version", version, "--locked", "--root", tools]);
}
run("rustup", ["target", "add", "wasm32-unknown-unknown"]);
run("cargo", ["build", "-p", "flow-like-device-crypto", "--features", "wasm", "--target", "wasm32-unknown-unknown", ...(debug ? [] : ["--release"])]);
const output = join(root, "apps/web/public/device-crypto");
mkdirSync(output, { recursive: true });
run(executable, [join(target, "wasm32-unknown-unknown", debug ? "debug" : "release", "flow_like_device_crypto.wasm"), "--target", "web", "--out-dir", output, "--out-name", "flow_like_device_crypto"]);
cpSync(output, join(root, "apps/desktop/public/device-crypto"), { recursive: true });
