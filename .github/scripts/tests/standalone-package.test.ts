import {
	afterAll,
	afterEach,
	beforeAll,
	describe,
	expect,
	test,
} from "bun:test";
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { createReadStream, existsSync, readdirSync, statSync } from "node:fs";
import { mkdtemp, open, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import AdmZip from "adm-zip";
import {
	buildStandalonePackage,
	verifyReleaseManifest,
} from "../../../packages/ui/lib/device-package";
import { standalonePackageFixture as fixture } from "./fixtures/standalone-package";
const originalFetch = globalThis.fetch;
afterEach(() => {
	globalThis.fetch = originalFetch;
});

describe("large native package startup", () => {
	let directory: string;
	let payload: string;
	let digest: string;
	const size = 256 * 1024 * 1024 + 1;
	beforeAll(async () => {
		directory = await mkdtemp(join(tmpdir(), "standalone-download-test-"));
		payload = join(directory, "runtime");
		const file = await open(payload, "w");
		try {
			await file.write(
				'#!/bin/sh\nprintf "%s\\n" "$*" >> "$RUN_LOG"\nif [ "$3" = enroll ]; then rm onboarding.json; fi\nexit 0\n',
			);
			await file.truncate(size);
		} finally {
			await file.close();
		}
		const hash = createHash("sha256");
		for await (const chunk of createReadStream(payload)) hash.update(chunk);
		digest = hash.digest("hex");
	});
	afterAll(async () => {
		await rm(directory, { recursive: true, force: true });
	});

	async function packageFixture() {
		const data = await fixture();
		data.manifest.artifacts[0] = {
			target: "aarch64-apple-darwin",
			url: "https://releases.example/runtime'$(touch$IFS'injected')'",
			size,
			sha256: digest,
		};
		const verified = await verifyReleaseManifest(
			await data.sign(data.manifest),
			data.config,
		);
		// Re-verification must discard changes to the caller's cached metadata.
		verified.manifest.artifacts[0].url =
			"https://untrusted.example/substituted";
		verified.manifest.artifacts[0].sha256 = "b".repeat(64);
		globalThis.fetch = (async () => {
			throw new Error(
				"Large binary packages must not download runtime bytes in the browser",
			);
		}) as unknown as typeof fetch;
		const blob = await buildStandalonePackage({
			...data.input,
			target: "aarch64-apple-darwin",
			mode: "binary",
			verifiedRelease: verified,
		});
		expect(blob.size).toBeLessThan(16 * 1024);
		const archive = new AdmZip(Buffer.from(await blob.arrayBuffer()));
		expect(archive.getEntry("flow-like-standalone")).toBeNull();
		expect(archive.readAsText("start.sh")).toContain(
			"sh ./download-runtime.sh",
		);
		const folder = await mkdtemp(join(directory, "package-"));
		archive.extractAllTo(folder, true);
		const curlPath = join(folder, "curl");
		await writeFile(
			curlPath,
			`#!/bin/sh
set -eu
printf '%s\\n' "$@" > "$CURL_LOG"
output=''
while [ "$#" -gt 0 ]; do
  if [ "$1" = --output ]; then output=$2; shift; fi
  shift
done
case "$DOWNLOAD_CASE" in
  error) exit 7 ;;
  unbounded) dd if=/dev/zero of="$output" bs=1 seek=2147483648 count=1 2>/dev/null ;;
  truncated) printf short > "$output" ;;
  *) cp "$PAYLOAD" "$output" ;;
esac
case "$DOWNLOAD_CASE" in
  tampered) printf x | dd of="$output" bs=1 count=1 conv=notrunc 2>/dev/null ;;
  oversized) printf x >> "$output" ;;
  partial) exit 18 ;;
  redirect) printf 302; exit 0 ;;
esac
printf 200
`,
			{ mode: 0o700 },
		);
		const runLog = join(folder, "run.log");
		const curlLog = join(folder, "curl.log");
		const run = async (downloadCase: string) => {
			const result = spawnSync("/bin/sh", ["./start.sh"], {
				cwd: folder,
				env: {
					PATH: `${folder}:/usr/bin:/bin:/usr/sbin:/sbin`,
					PAYLOAD: payload,
					DOWNLOAD_CASE: downloadCase,
					RUN_LOG: runLog,
					CURL_LOG: curlLog,
				},
				encoding: "utf8",
				timeout: 20_000,
			});
			return {
				exit: result.status,
				stdout: result.stdout,
				stderr: result.stderr,
				error: result.error,
			};
		};
		return { folder, run, runLog, curlLog };
	}

	test("downloads to disk, verifies the signed bytes, then enrolls and runs; restarts reuse the binary", async () => {
		const data = await packageFixture();
		const result = await data.run("success");
		expect(result).toMatchObject({ exit: 0, stderr: "" });
		expect(await Bun.file(data.runLog).text()).toBe(
			"--state-dir ./state enroll .\n--state-dir ./state run\n",
		);
		const args = (await Bun.file(data.curlLog).text()).trim().split("\n");
		expect(args[0]).toBe("--disable");
		expect(args[args.indexOf("--url") + 1]).toBe(
			"https://releases.example/runtime'$(touch$IFS'injected')'",
		);
		expect(args[args.indexOf("--proto") + 1]).toBe("=https");
		expect(args[args.indexOf("--max-filesize") + 1]).toBe(String(size));
		expect(args[args.indexOf("--max-redirs") + 1]).toBe("0");
		expect(args[args.indexOf("--max-time") + 1]).toBe(
			String(300 + Math.floor(size / (32 * 1024))),
		);
		expect(args[args.indexOf("--speed-time") + 1]).toBe("60");
		expect(args).not.toContain("--location");
		expect(existsSync(join(data.folder, "injected"))).toBe(false);
		expect(
			statSync(join(data.folder, "flow-like-standalone")).mode & 0o777,
		).toBe(0o700);
		expect(
			readdirSync(data.folder).some((name) =>
				name.startsWith(".runtime-download."),
			),
		).toBe(false);
		expect((await data.run("error")).exit).toBe(0);
	}, 30_000);

	test("a preexisting unverified binary cannot enroll, but an enrolled runtime can update", async () => {
		const data = await packageFixture();
		const binary = join(data.folder, "flow-like-standalone");
		await writeFile(binary, '#!/bin/sh\nprintf changed >> "$RUN_LOG"\n', {
			mode: 0o700,
		});
		expect((await data.run("error")).exit).not.toBe(0);
		expect(existsSync(data.runLog)).toBe(false);
		await rm(join(data.folder, "onboarding.json"));
		expect((await data.run("error")).exit).toBe(0);
		expect(await Bun.file(data.runLog).text()).toBe("changed");
	});

	for (const failure of [
		"tampered",
		"truncated",
		"oversized",
		"partial",
		"redirect",
		"error",
		"unbounded",
	])
		test(`${failure} download leaves no executable and never enrolls or starts`, async () => {
			const data = await packageFixture();
			expect((await data.run(failure)).exit).not.toBe(0);
			expect(existsSync(join(data.folder, "flow-like-standalone"))).toBe(false);
			expect(existsSync(data.runLog)).toBe(false);
			expect(
				readdirSync(data.folder).some((name) =>
					name.startsWith(".runtime-download."),
				),
			).toBe(false);
		}, 30_000);
});
