import { spawnSync } from "node:child_process";
import { delimiter, resolve } from "node:path";

const desktop = resolve(import.meta.dir, "..");
const workspace = resolve(desktop, "../..");
const android = resolve(desktop, "src-tauri/gen/android");
const options = process.argv.slice(2);
for (let index = 0; index < options.length; index++) {
	const option = options[index];
	if (option === "--config" || option === "-c") {
		if (!options[++index] || options[index].startsWith("-")) {
			throw new Error(`${option} requires a configuration file or JSON value`);
		}
	} else if (option !== "--verbose" && !/^-v+$/.test(option)) {
		throw new Error(
			`Unsupported release option: ${option}. This command builds signed ARM64 release artifacts; only --config and --verbose can be overridden.`,
		);
	}
}
const environment = {
	...process.env,
	ANDROID_REQUIRE_SIGNING: "1",
	ORT_LIB_LOCATION:
		process.env.ORT_LIB_LOCATION ??
		resolve(desktop, "src-tauri/thirdparty/onnxruntime/android/arm64-v8a"),
	ORT_PREFER_DYNAMIC_LINK: "1",
	PATH: `${resolve(workspace, "tools/cargo-queue/bin")}${delimiter}${process.env.PATH ?? ""}`,
};

function run(command: string, args: string[], cwd: string) {
	const result = spawnSync(command, args, {
		cwd,
		env: environment,
		stdio: "inherit",
	});
	if (result.error) throw result.error;
	if (result.status !== 0) process.exit(result.status ?? 1);
}

run(
	"bun",
	[
		"run",
		"tauri",
		"android",
		"build",
		"--target",
		"aarch64",
		"--aab",
		"--ci",
		...options,
	],
	desktop,
);

// Tauri has just built and staged the Rust library. Reuse it when fusing the APK.
run(
	process.platform === "win32" ? "gradlew.bat" : "./gradlew",
	[
		":app:packageStandaloneUniversalRelease",
		"-x",
		":app:rustBuildUniversalRelease",
		"-PabiList=arm64-v8a",
		"-ParchList=arm64",
		"-PtargetList=aarch64",
		"--console=plain",
	],
	android,
);
