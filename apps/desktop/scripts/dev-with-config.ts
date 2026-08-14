import { spawn, spawnSync } from "child_process";
import { arch, platform } from "os";

function getConfigPath(): string {
	const osType = platform();
	const architecture = arch();

	let configPath = "";

	switch (osType) {
		case "darwin": // macOS
			configPath =
				architecture === "arm64"
					? "src-tauri/configs/tauri.macos.arm.conf.json"
					: "src-tauri/configs/tauri.macos.intel.conf.json";
			break;
		case "win32": // Windows
			configPath =
				architecture === "arm64"
					? "src-tauri/configs/tauri.win.arm.conf.json"
					: "src-tauri/configs/tauri.win.x64.conf.json";
			break;
		case "linux": // Linux
			configPath = "src-tauri/configs/tauri.linux.x64.conf.json";
			break;
		default:
			throw new Error(`Unsupported platform: ${osType}`);
	}

	return configPath;
}

function prepareWindowsPrereqs(): void {
	if (platform() !== "win32") return;

	const vcArch = arch() === "arm64" ? "arm64" : "x64";
	console.log(`Preparing Windows VC runtime DLLs for ${vcArch}...`);

	const result = spawnSync(
		"bun",
		["./scripts/prepare-windows-prereqs.ts", "--arch", vcArch],
		{ stdio: "inherit" },
	);

	if (result.error || result.status !== 0) {
		const reason =
			result.error?.message ??
			(result.signal
				? `signal ${result.signal}`
				: `exit code ${result.status}`);
		throw new Error(`Failed to prepare Windows prerequisites: ${reason}`);
	}
}

async function main() {
	try {
		if (platform() === "darwin") {
			process.env.ORT_LIB_LOCATION ??= `${process.cwd()}/src-tauri/gen/apple/thirdparty/onnxruntime.xcframework/macos-arm64_x86_64`;
			process.env.MACOSX_DEPLOYMENT_TARGET ??= "14.0";
		}

		const configPath = getConfigPath();
		console.log(`Detected OS: ${platform()}, Architecture: ${arch()}`);
		console.log(`Using config: ${configPath}`);
		prepareWindowsPrereqs();

		console.log(`Starting Tauri dev with config...`);

		const tauriDev = spawn(
			"bun",
			[
				"run",
				"tauri",
				"dev",
				"+nightly",
				"-d",
				"-b",
				"none",
				"--config",
				configPath,
			],
			{
				stdio: "inherit",
			},
		);

		process.on("SIGINT", () => {
			console.log("\nShutting down...");
			tauriDev.kill();
			process.exit(0);
		});
	} catch (error) {
		console.error("Error:", error);
		process.exit(1);
	}
}

main();
