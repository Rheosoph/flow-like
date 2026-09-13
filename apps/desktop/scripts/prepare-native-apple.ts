import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const desktop = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const tauri = path.join(desktop, "src-tauri");
const project = path.join(tauri, "apple/FlowLikeNative/FlowLikeNative.xcodeproj");
const configuration = process.argv.includes("--release") ? "Release" : "Debug";
const derived = path.join(tauri, "target/native-apple");
const output = path.join(tauri, "binaries/apple-native");

if (process.platform !== "darwin") throw new Error("Apple native integrations require Xcode on macOS.");
const resolved = spawnSync("python3", [
	path.join(desktop, "scripts/apple-bundle-version.py"),
	"--config", path.join(tauri, "tauri.conf.json"), "--platform", "macOS",
], { encoding: "utf8" });
if (resolved.error) throw resolved.error;
if (resolved.status !== 0) throw new Error(`Resolving Apple bundle versions failed: ${resolved.stderr}`);
const versions = JSON.parse(resolved.stdout) as { CFBundleShortVersionString: string; CFBundleVersion: string };
const result = spawnSync("xcodebuild", [
	"-quiet", "-project", project, "-scheme", "FlowLikeNativeMac",
	"-configuration", configuration, "-destination", "generic/platform=macOS",
	"-derivedDataPath", derived, "CODE_SIGNING_ALLOWED=NO", "ENABLE_DEBUG_DYLIB=NO",
	`MARKETING_VERSION=${versions.CFBundleShortVersionString}`,
	`CURRENT_PROJECT_VERSION=${versions.CFBundleVersion}`, "build",
], { cwd: desktop, stdio: "inherit" });
if (result.error) throw result.error;
if (result.status !== 0) throw new Error(`Apple native build failed (${result.status}).`);
const products = path.join(derived, "Build/Products", configuration);
const verified = spawnSync("python3", [
	path.join(desktop, "scripts/verify-apple-intents.py"),
	path.join(products, "FlowLikeNative.framework/Resources/Metadata.appintents/extract.actionsdata"),
	"--module", "FlowLikeNative",
], { stdio: "inherit" });
if (verified.error) throw verified.error;
if (verified.status !== 0) throw new Error("Native App Shortcuts metadata is incomplete.");
fs.mkdirSync(output, { recursive: true });
for (const [source, target] of [
	["FlowLikeNative.framework", "FlowLikeNative.framework"],
	["FlowLikeWidgets_macOS.appex", "FlowLikeWidgets.appex"],
	["FlowLikeShare_macOS.appex", "FlowLikeShare.appex"],
	["FlowLikeNative.framework/Resources/Metadata.appintents", "Metadata.appintents"],
]) {
	const destination = path.join(output, target);
	fs.rmSync(destination, { recursive: true, force: true });
	fs.cpSync(path.join(products, source), destination, { recursive: true, verbatimSymlinks: true });
}
// Extensions need their own entitlements, so sign them before Tauri signs the outer bundle.
const identity = process.env.APPLE_SIGNING_IDENTITY;
for (const [bundle, entitlements] of [
	["FlowLikeNative.framework", undefined],
	["FlowLikeWidgets.appex", "Extensions/Widgets/Widgets-macOS.entitlements"],
	["FlowLikeShare.appex", "Extensions/Share/Share-macOS.entitlements"],
] as const) {
	const args = ["--force", "--sign", identity || "-", "--timestamp=none"];
	if (identity) { args.splice(args.indexOf("--timestamp=none"), 1, "--timestamp"); args.push("--options", "runtime"); }
	if (entitlements) args.push("--entitlements", path.join(tauri, "apple/FlowLikeNative", entitlements));
	args.push(path.join(output, bundle));
	const signed = spawnSync("codesign", args, { stdio: "inherit" });
	if (signed.error) throw signed.error;
	if (signed.status !== 0) throw new Error(`Signing ${bundle} failed.`);
}
console.log(`Prepared Apple native integrations (${configuration}).`);
