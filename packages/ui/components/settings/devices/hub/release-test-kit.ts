import type {
	ReleaseTarget,
	StandaloneRelease,
} from "../../../../lib/device-package";
import type { FakeWorkspace } from "../testing/fake-workspace";

/*
 * Signed agent release lists for tests, with the validity a test chooses. The
 * shared fake publishes one list with one lifetime; a test that depends on
 * how long a list lives signs its own here and serves it from the fake hub.
 */

export const DAY_S = 86_400;
export const TEST_RELEASE_URL =
	"https://releases.flow-like.com/standalone/stable/release.jws";

const encoder = new TextEncoder();
const base64url = (bytes: Uint8Array) =>
	btoa(String.fromCharCode(...bytes))
		.replaceAll("+", "-")
		.replaceAll("/", "_")
		.replace(/=+$/, "");

export interface TestReleaseOptions {
	version?: string;
	sequence?: number;
	/** The hub's minimum release number. */
	minimum?: number;
	/** Seconds before the fake clock the list was issued. */
	issuedAgoS?: number;
	/** Seconds from the fake clock to the list's end; negative for a list that has run out. */
	endsInS?: number;
	artifacts?: readonly { target: ReleaseTarget; size: number }[];
	/** Platforms of the Docker image; `null` for a release without one. */
	platforms?: string[] | null;
}

export interface ReleaseSigner {
	/** The key the hub pins, as its release trust states it. */
	publicKey: string;
	/** Signs a list as the client verifies it (a real Ed25519 signature). */
	sign(manifest: StandaloneRelease): Promise<string>;
}

export async function releaseSigner(): Promise<ReleaseSigner> {
	const pair = (await crypto.subtle.generateKey("Ed25519", true, [
		"sign",
		"verify",
	])) as CryptoKeyPair;
	const publicKey = base64url(
		new Uint8Array(await crypto.subtle.exportKey("raw", pair.publicKey)),
	);
	const kid = base64url(
		new Uint8Array(
			await crypto.subtle.digest(
				"SHA-256",
				encoder.encode(
					JSON.stringify({ crv: "Ed25519", kty: "OKP", x: publicKey }),
				),
			),
		),
	);
	const header = base64url(
		encoder.encode(
			JSON.stringify({
				alg: "EdDSA",
				typ: "flow-like-standalone-release+jws",
				kid,
			}),
		),
	);
	return {
		publicKey,
		sign: async (manifest) => {
			const signed = `${header}.${base64url(encoder.encode(JSON.stringify(manifest)))}`;
			const signature = await crypto.subtle.sign(
				"Ed25519",
				pair.privateKey,
				encoder.encode(signed),
			);
			return `${signed}.${base64url(new Uint8Array(signature))}`;
		},
	};
}

/** The sample fleet's release (0.9.4, release number 44), valid for a year unless the test says otherwise. */
export function testManifest(
	nowS: number,
	options: TestReleaseOptions = {},
): StandaloneRelease {
	const version = options.version ?? "0.9.4";
	const artifacts = options.artifacts ?? [
		{ target: "x86_64-unknown-linux-gnu" as const, size: 1_024 },
	];
	const platforms = options.platforms ?? null;
	return {
		version: 1,
		state_schema_version: 12,
		sequence: options.sequence ?? 44,
		release_version: version,
		issued_at: nowS - (options.issuedAgoS ?? 3_600),
		expires_at: nowS + (options.endsInS ?? 365 * DAY_S),
		artifacts: artifacts.map(({ target, size }) => ({
			target,
			url: `https://releases.flow-like.com/standalone/${version}/${target}`,
			size,
			sha256: "0".repeat(64),
		})),
		container: platforms
			? {
					image: `ghcr.io/tm9657/flow-like-standalone@sha256:${"2b".repeat(32)}`,
					platforms,
				}
			: null,
	};
}

export interface PublishedTestRelease {
	manifest: StandaloneRelease;
	jws: string;
	/** Serves another list under the same key and the same hub settings, as a renewal or a new release would. */
	republish(options?: TestReleaseOptions): Promise<StandaloneRelease>;
	/** Serves these bytes at the release address, whatever they are. */
	serve(jws: string): void;
}

/** Publishes a release on the fake hub and makes the hub pin its key. */
export async function publishTestRelease(
	fake: Pick<FakeWorkspace, "hub" | "clock">,
	options: TestReleaseOptions = {},
	signer?: ReleaseSigner,
): Promise<PublishedTestRelease> {
	const key = signer ?? (await releaseSigner());
	const nowS = () => Math.floor(fake.clock.now() / 1000);
	const serve = (jws: string) => {
		fake.hub.release = { url: TEST_RELEASE_URL, jws };
	};
	const publish = async (next: TestReleaseOptions) => {
		const manifest = testManifest(nowS(), next);
		const jws = await key.sign(manifest);
		serve(jws);
		return { manifest, jws };
	};
	const first = await publish(options);
	fake.hub.releaseTrust = {
		manifest_url: TEST_RELEASE_URL,
		public_keys: [key.publicKey],
		minimum_sequence: options.minimum ?? 41,
	};
	return {
		...first,
		republish: async (next = options) => (await publish(next)).manifest,
		serve,
	};
}
