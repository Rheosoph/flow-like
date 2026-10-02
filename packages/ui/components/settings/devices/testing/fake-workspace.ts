import { setSystemTime } from "bun:test";
import { QueryClient } from "@tanstack/react-query";
import type {
	CertificateAuthorityEnvelope,
	CertificateAuthorityPublic,
	LocalCertificateAuthority,
} from "../../../../lib/device-management/certificate-authority";
import {
	base64url,
	unbase64url,
} from "../../../../lib/device-management/crypto";
import { backupSourceDigest } from "../../../../lib/device-management/recovery";
import {
	type AccountRecoveryState,
	type DeviceAccountScope,
	type LocalDeviceVault,
	accountStorageKey,
	deviceIdentityKey,
} from "../../../../lib/device-management/storage";
import type {
	ArchiveRoster,
	BrowserController,
	BrowserMlsEndpoint,
	ControllerPublic,
	DeviceCrypto,
	DeviceReceipt,
	Ed25519PublicKey,
	FleetAudience,
	FleetTrustedContext,
	FleetView,
	ManagementPolicy,
	NoiseHandshake,
	OnboardingManifest,
	TelemetryRoster,
} from "../../../../lib/device-management/types";
import { ACTIVITY_STORAGE_PREFIX } from "../../../../lib/device-management/workspace/activity";
import {
	type DeviceWorkspaceOptions,
	type DeviceWorkspaceRuntime,
	createDeviceWorkspace,
	dismissWorkspaceSwitch,
	disposeDeviceWorkspace,
	lastWorkspaceSwitch,
	openDeviceWorkspace,
	registerDeviceWorkspace,
} from "../../../../lib/device-management/workspace/registry";
import type {
	UnlockOptions,
	WorkspaceDeps,
} from "../../../../lib/device-management/workspace/types";
import type { IProfile } from "../../../../types";
import {
	type DeviceSeed,
	FAKE_PASSWORD,
	type FakeAgent,
	type FakeDeviceApi,
	type FakeDeviceApiOptions,
	type FakeHub,
	type FakeVaultSecret,
	fakeBytes,
	fakeCompact,
	fakeDeviceApi,
	fakeDigest,
	fakeKey,
	fakeKeys,
	fakePasswordCheck,
	openFakeRecovery,
	openFakeVault,
	readFakeCompact,
	sealFakeRecovery,
	sealFakeVault,
} from "./fake-device-api";

/*
 * The real workspace managers over a fake hub, fake agents, fake crypto, a fake
 * clock and an in-memory browser store (plan §2.4 "Testing support").
 *
 *   const fake = await createFakeWorkspace();   // golden sample fleet, as the prototype shows it
 *   fake.workspace.keys.snapshot(id);            // real KeySessionManager
 *   fake.api.calls; fake.api.agent(id);          // what was sent; what the device answers
 *   await fake.dispose();                        // in afterEach: restores every global
 *
 * While a fake workspace is alive `Date.now()` is the fake clock, and
 * `indexedDB`, `navigator.locks` and `WebSocket` are in-memory fakes.
 */

const utf8 = (text: string) => new TextEncoder().encode(text);
const FACTS_PREFIX = "flow-like.device-facts.";

/* Clock. */

export interface FakeClock {
	/** Epoch milliseconds. */
	now(): number;
	set(ms: number): void;
	advance(ms: number): void;
	restore(): void;
}

/** `system` also moves `Date.now()`; timers keep running in real time. */
export function createFakeClock(startMs: number, system = true): FakeClock {
	let current = startMs;
	const sync = () => {
		if (system) setSystemTime(new Date(current));
	};
	sync();
	return {
		now: () => current,
		set(ms) {
			current = ms;
			sync();
		},
		advance(ms) {
			current += ms;
			sync();
		},
		restore() {
			if (system) setSystemTime();
		},
	};
}

/* Browser stores: IndexedDB, Web Locks, storage persistence. */

interface KeyRange {
	lower: string;
	upper: string;
}

type Rows = Map<string, unknown>;

function inRange(key: string, range: KeyRange | undefined) {
	return !range || (key >= range.lower && key <= range.upper);
}

function idbRequest<T>(result: T) {
	const request = {
		result,
		error: null as { name: string } | null,
		onsuccess: undefined as (() => void) | undefined,
		onerror: undefined as
			| ((event: { preventDefault(): void }) => void)
			| undefined,
	};
	queueMicrotask(() => request.onsuccess?.());
	return request;
}

/** Rows are staged per transaction and committed once its last request settled. */
function fakeIndexedDb(stores: Map<string, Rows>, failCommit: () => boolean) {
	const transaction = (names: string | string[]) => {
		const staged = new Map(
			[names].flat().map((name) => [name, new Map(stores.get(name) ?? [])]),
		);
		let aborted = false;
		let pending = 0;
		const settle = () =>
			queueMicrotask(() => {
				pending -= 1;
				if (pending > 0 || aborted) return;
				if (failCommit()) {
					tx.abort();
					return;
				}
				for (const [name, rows] of staged) stores.set(name, rows);
				tx.oncomplete?.();
			});
		const track = <T>(result: T) => {
			pending += 1;
			const request = idbRequest(result);
			queueMicrotask(() => queueMicrotask(settle));
			return request;
		};
		const objectStore = (name: string) => {
			const rows = staged.get(name);
			if (!rows) throw new Error(`Store ${name} is outside this transaction`);
			const keys = (range?: KeyRange) =>
				[...rows.keys()].filter((key) => inRange(key, range)).sort();
			return {
				get: (key: string) => track(structuredClone(rows.get(key))),
				getAll: (range?: KeyRange) =>
					track(keys(range).map((key) => structuredClone(rows.get(key)))),
				getAllKeys: (range?: KeyRange) => track(keys(range)),
				put(value: unknown, key: string) {
					rows.set(key, structuredClone(value));
					return track(undefined);
				},
				add(value: unknown, key: string) {
					const request = track(undefined);
					if (!rows.has(key)) {
						rows.set(key, structuredClone(value));
						return request;
					}
					request.error = { name: "ConstraintError" };
					queueMicrotask(() => {
						request.onerror?.({ preventDefault() {} });
						tx.abort();
					});
					return request;
				},
				delete(key: string | KeyRange) {
					for (const row of [...rows.keys()])
						if (typeof key === "string" ? row === key : inRange(row, key))
							rows.delete(row);
					return track(undefined);
				},
			};
		};
		const tx = {
			oncomplete: undefined as (() => void) | undefined,
			onerror: undefined as (() => void) | undefined,
			onabort: undefined as (() => void) | undefined,
			abort() {
				aborted = true;
				queueMicrotask(() => tx.onabort?.());
			},
			objectStore,
		};
		pending += 1;
		queueMicrotask(settle);
		return tx;
	};
	return { open: () => idbRequest({ close() {}, transaction }) };
}

interface HeldLock {
	reject(error: unknown): void;
}

export interface FakeLocks {
	readonly held: Map<string, HeldLock>;
	request(
		name: string,
		options: { ifAvailable?: boolean; steal?: boolean },
		run: (lock: object | null) => Promise<void>,
	): Promise<void>;
	query(): Promise<{ held: { name: string }[] }>;
	/** Another window takes the lock (stealing it); the returned function makes that window let go. */
	takeElsewhere(name: string): () => void;
}

function fakeLocks(): FakeLocks {
	const held = new Map<string, HeldLock>();
	const locks: FakeLocks = {
		held,
		request(name, options, run) {
			return new Promise<void>((resolve, reject) => {
				const current = held.get(name);
				if (current && !options.steal) {
					run(null).then(resolve, reject);
					return;
				}
				if (current) {
					held.delete(name);
					current.reject(new DOMException("Lock stolen", "AbortError"));
				}
				const entry: HeldLock = { reject };
				held.set(name, entry);
				run({ name }).then(() => {
					if (held.get(name) === entry) held.delete(name);
					resolve();
				}, reject);
			});
		},
		async query() {
			return { held: [...held.keys()].map((name) => ({ name })) };
		},
		takeElsewhere(name) {
			let release!: () => void;
			void locks
				.request(
					name,
					{ steal: true },
					() =>
						new Promise<void>((resolve) => {
							release = resolve;
						}),
				)
				.catch(() => undefined);
			return () => release();
		},
	};
	return locks;
}

function memoryStorage(): Storage {
	const rows = new Map<string, string>();
	return {
		get length() {
			return rows.size;
		},
		clear: () => rows.clear(),
		getItem: (key) => rows.get(key) ?? null,
		key: (index) => [...rows.keys()][index] ?? null,
		removeItem: (key) => {
			rows.delete(key);
		},
		setItem: (key, value) => {
			rows.set(key, String(value));
		},
	};
}

function replaceProperty(
	target: object,
	key: string,
	value: unknown,
): () => void {
	const previous = Object.getOwnPropertyDescriptor(target, key);
	Object.defineProperty(target, key, {
		configurable: true,
		writable: true,
		value,
	});
	return () => {
		if (previous) Object.defineProperty(target, key, previous);
		else Reflect.deleteProperty(target, key);
	};
}

export type FakePersistence = "persisted" | "denied" | "unavailable";

export interface FakeBrowser {
	/** IndexedDB object stores by name (`vaults`, `recovery`, `fleet`, `snapshots`, `authorities`). */
	readonly stores: Map<string, Rows>;
	readonly locks: FakeLocks;
	/** What `navigator.storage` reports; change it between reads. */
	persistence: FakePersistence;
	/** The next IndexedDB transaction aborts at commit. */
	failNextCommit(): void;
	restore(): void;
}

function fakeStorageManager(browser: Pick<FakeBrowser, "persistence">) {
	return {
		persisted: async () => {
			if (browser.persistence === "unavailable") throw new Error("unavailable");
			return browser.persistence === "persisted";
		},
		persist: async () => browser.persistence === "persisted",
	};
}

/** Installs the fake browser stores as globals; `restore()` puts the previous ones back. */
export function installFakeBrowser(
	options: {
		persistence?: FakePersistence;
		WebSocket?: typeof WebSocket;
		fetch?: typeof fetch;
	} = {},
): FakeBrowser {
	const stores = new Map<string, Rows>();
	const locks = fakeLocks();
	let failCommit = false;
	const restores: (() => void)[] = [];
	const browser: FakeBrowser = {
		stores,
		locks,
		persistence: options.persistence ?? "persisted",
		failNextCommit() {
			failCommit = true;
		},
		restore() {
			for (const restore of restores.reverse()) restore();
			restores.length = 0;
		},
	};
	const consumeFailure = () => {
		const fail = failCommit;
		failCommit = false;
		return fail;
	};
	restores.push(
		replaceProperty(
			globalThis,
			"indexedDB",
			fakeIndexedDb(stores, consumeFailure),
		),
		replaceProperty(globalThis, "IDBKeyRange", {
			bound: (lower: string, upper: string) => ({ lower, upper }),
		}),
	);
	if (!globalThis.navigator)
		restores.push(replaceProperty(globalThis, "navigator", {}));
	restores.push(
		replaceProperty(globalThis.navigator, "locks", locks),
		replaceProperty(
			globalThis.navigator,
			"storage",
			fakeStorageManager(browser),
		),
	);
	if (!globalThis.localStorage)
		restores.push(replaceProperty(globalThis, "localStorage", memoryStorage()));
	if (options.WebSocket)
		restores.push(replaceProperty(globalThis, "WebSocket", options.WebSocket));
	if (options.fetch)
		restores.push(replaceProperty(globalThis, "fetch", options.fetch));
	return browser;
}

/* Crypto. */

export interface FakeController extends BrowserController {
	/** `close()` was called. */
	readonly closed: boolean;
	/** `free()` was called. */
	readonly freed: boolean;
	/** The owner's invitation key is attached for signing. */
	readonly holdsInvitation: boolean;
}

export interface FakeCrypto extends DeviceCrypto {
	/** Every controller opened through `unlockControllerVault`, oldest first. */
	readonly controllers: FakeController[];
	/** How often `default()` (the module's init) ran. */
	readonly loads: number;
}

export interface FakeCryptoOptions {
	/** `false`: a crypto bundle built before the held-signer methods. */
	heldSigner?: boolean;
}

const WRONG_PASSWORD = "The vault cannot be opened with this password.";

function openVault(
	bytes: Uint8Array,
	kind: FakeVaultSecret["fake"],
	deviceId: string,
	password: Uint8Array,
): FakeVaultSecret {
	let secret: FakeVaultSecret;
	try {
		secret = openFakeVault(bytes);
	} catch {
		throw new Error(WRONG_PASSWORD);
	}
	if (
		secret.fake !== kind ||
		secret.device_id !== deviceId ||
		secret.check !== fakePasswordCheck(deviceId, password)
	)
		throw new Error(WRONG_PASSWORD);
	return secret;
}

function fakeMlsEndpoint(): BrowserMlsEndpoint {
	const checkpoint = {
		store_id: Array.from(fakeBytes("mls-store")),
		revision: 1,
		digest: fakeDigest("mls-checkpoint"),
	};
	return {
		deliveryReceipts: () => [],
		prepareReceiptConfirmation() {},
		position: () => ({ joined: false, retired: false, sequence: 0 }),
		preparedSnapshot: () => ({
			previous_checkpoint: null,
			checkpoint,
			snapshot: { store_id: checkpoint.store_id, revision: 1, ciphertext: "" },
		}),
		checkpoint: () => checkpoint,
		confirmCommit: () => ({ kind: "none" }),
		discardPrepared() {},
		prepareKeyPackage() {},
		prepareJoin() {},
		prepareReceive() {},
		close() {},
		free() {},
	};
}

function fakeHandshake(
	grantId: string,
	controllerKey: Ed25519PublicKey,
): NoiseHandshake {
	const sessionId = crypto.randomUUID();
	const copy = (bytes: Uint8Array) => Uint8Array.from(bytes);
	return {
		certificate: () =>
			fakeCompact(
				{ grant_id: grantId, controller_key: controllerKey },
				controllerKey,
			),
		sessionId: () => sessionId,
		write: () => fakeBytes(`noise:${sessionId}`),
		read() {},
		finish: () => ({ encrypt: copy, decrypt: copy, close() {}, free() {} }),
		close() {},
		free() {},
	};
}

interface ReaderDeclaration {
	api_base_url: string;
	user_id: string;
	device_id: string;
	controller_key: Ed25519PublicKey;
	revision: number;
	issued_at: number;
	expires_at: number;
}

interface FleetManifest {
	device_id: string;
	audience: FleetAudience;
}

function freshVault(
	secret: FakeVaultSecret,
	deviceId: string,
	password: Uint8Array,
	controller?: ControllerPublic,
): number[] {
	return Array.from(
		sealFakeVault({
			...secret,
			device_id: deviceId,
			check: fakePasswordCheck(deviceId, password),
			...(controller ? { controller } : {}),
			nonce: crypto.randomUUID(),
		}),
	);
}

function createController(
	secret: FakeVaultSecret,
	options: FakeCryptoOptions,
): FakeController {
	let bundle = secret.controller as ControllerPublic;
	let closed = false;
	let freed = false;
	let held: Ed25519PublicKey | undefined;
	const alive = () => {
		if (freed) throw new Error("The controller was used after free().");
	};
	const reader = (
		trusted: FleetTrustedContext,
		receipt: DeviceReceipt,
		readerJws: string,
	) => {
		alive();
		const declared = readFakeCompact<ReaderDeclaration>(
			readerJws,
			bundle.controller_key,
		);
		if (
			declared.device_id !== receipt.device_id ||
			declared.api_base_url !== trusted.api_base_url ||
			declared.user_id !== trusted.user_id
		)
			throw new Error("The reader declaration belongs to another device.");
		return declared;
	};
	const manifestOf = (receipt: DeviceReceipt, manifestJws: string) => {
		const manifest = readFakeCompact<FleetManifest>(
			manifestJws,
			receipt.identity.telemetry_key,
		);
		if (manifest.device_id !== receipt.device_id)
			throw new Error("The snapshot belongs to another device.");
		return manifest;
	};
	const signHeld = <T extends { device_id: string }>(payload: T) => {
		alive();
		if (!held) throw new Error("No owner key is attached.");
		if (payload.device_id !== bundle.device_id)
			throw new Error("The owner key belongs to another device.");
		return fakeCompact(payload, held);
	};
	const controller: FakeController = {
		get closed() {
			return closed;
		},
		get freed() {
			return freed;
		},
		get holdsInvitation() {
			return held !== undefined;
		},
		publicBundle: () => {
			alive();
			return bundle;
		},
		verifyFleetReader: (trusted, receipt, readerJws) => {
			const { revision, issued_at, expires_at } = reader(
				trusted,
				receipt,
				readerJws,
			);
			return { revision, issued_at, expires_at };
		},
		createFleetReader: (api, user, revision, issuedAt, expiresAt) => {
			alive();
			const declaration: ReaderDeclaration = {
				api_base_url: api,
				user_id: user,
				device_id: bundle.device_id,
				controller_key: bundle.controller_key,
				revision: Number(revision),
				issued_at: Number(issuedAt),
				expires_at: Number(expiresAt),
			};
			return fakeCompact(declaration, bundle.controller_key);
		},
		verifyFleetView: (trusted, receipt, view: FleetView, now) => {
			const declared = reader(trusted, receipt, view.reader_jws);
			if (declared.expires_at <= Number(now))
				throw new Error("The reader declaration has expired.");
			return {
				reader_revision: declared.revision,
				reader_expires_at: declared.expires_at,
				audiences: view.snapshots.map(
					(snapshot) => manifestOf(receipt, snapshot.manifest_jws).audience,
				),
			};
		},
		openFleet: (_trusted, receipt, _view, snapshot) => {
			alive();
			manifestOf(receipt, snapshot.manifest_jws);
			return unbase64url(snapshot.ciphertext, 1024 * 1024);
		},
		sealInventory: (binding, plaintext) => {
			alive();
			return { binding, ciphertext: base64url(plaintext) };
		},
		openInventory: (expected, encrypted) => {
			alive();
			if (JSON.stringify(expected) !== JSON.stringify(encrypted.binding))
				throw new Error("Saved inventory was sealed for another binding.");
			return unbase64url(encrypted.ciphertext, 1024 * 1024);
		},
		openArchive: (_pins, archive) => {
			alive();
			return unbase64url(archive.ciphertext, 1024 * 1024);
		},
		freshEndpointVault: (password) => {
			alive();
			openVault(
				sealFakeVault(secret),
				"controller",
				bundle.device_id,
				password,
			);
			const endpointId = `endpoint-${crypto.randomUUID().slice(0, 8)}`;
			bundle = {
				...bundle,
				endpoint_id: endpointId,
				telemetry_member: {
					endpoint_id: endpointId,
					signing_key: fakeKey(`member:${bundle.device_id}:${endpointId}`),
				},
			};
			return {
				public_bundle: bundle,
				vault: freshVault(secret, bundle.device_id, password, bundle),
			};
		},
		signOnboardingManifest: (manifest) => fakeCompact(manifest),
		completeOnboarding: (manifest, password, invitationVault) => {
			alive();
			const invitation = openVault(
				invitationVault,
				"invitation",
				bundle.device_id,
				password,
			);
			const next = { ...bundle, device_id: manifest.device_id };
			return {
				controller: {
					public_bundle: next,
					vault: freshVault(secret, manifest.device_id, password, next),
				},
				invitation_vault: freshVault(invitation, manifest.device_id, password),
				manifest_jws: fakeCompact(manifest),
			};
		},
		beginNoise: (grantId) => {
			alive();
			return fakeHandshake(grantId, bundle.controller_key);
		},
		createTelemetry: () => fakeMlsEndpoint(),
		openTelemetry: () => fakeMlsEndpoint(),
		close() {
			closed = true;
			held = undefined;
		},
		free() {
			freed = true;
			held = undefined;
		},
	};
	if (options.heldSigner === false) return controller;
	return Object.assign(controller, {
		attachInvitation(password: Uint8Array, invitationVault: Uint8Array) {
			alive();
			held = openVault(
				invitationVault,
				"invitation",
				bundle.device_id,
				password,
			).public_key;
		},
		detachInvitation() {
			held = undefined;
		},
		signManagementPolicyHeld: (policy: ManagementPolicy) => signHeld(policy),
		signTelemetryRosterHeld: (roster: TelemetryRoster) => signHeld(roster),
		signArchiveRosterHeld: (roster: ArchiveRoster) => signHeld(roster),
	});
}

interface AuthorityVault {
	fake: "authority";
	authority_id: string;
	account_binding: string;
	check: string;
	public_bundle: CertificateAuthorityPublic;
	nonce?: string;
}

const PEM = (label: string) =>
	`-----BEGIN CERTIFICATE-----\n${base64url(fakeBytes(label))}\n-----END CERTIFICATE-----\n`;

function sealAuthority(
	publicBundle: CertificateAuthorityPublic,
	password: Uint8Array,
): number[] {
	const vault: AuthorityVault = {
		fake: "authority",
		authority_id: publicBundle.authority_id,
		account_binding: publicBundle.account_binding,
		check: fakePasswordCheck(publicBundle.authority_id, password),
		public_bundle: publicBundle,
		nonce: crypto.randomUUID(),
	};
	return Array.from(utf8(JSON.stringify(vault).padEnd(96, " ")));
}

function openAuthority(
	accountBinding: string,
	authorityId: string,
	password: Uint8Array,
	vault: Uint8Array,
): AuthorityVault {
	const opened = JSON.parse(new TextDecoder().decode(vault)) as AuthorityVault;
	if (
		opened.fake !== "authority" ||
		opened.authority_id !== authorityId ||
		opened.account_binding !== accountBinding ||
		opened.check !== fakePasswordCheck(authorityId, password)
	)
		throw new Error(WRONG_PASSWORD);
	return opened;
}

function authorityEnvelope(
	publicBundle: CertificateAuthorityPublic,
	password: Uint8Array,
): CertificateAuthorityEnvelope {
	return {
		public_bundle: publicBundle,
		vault: sealAuthority(publicBundle, password),
		root_vault: sealAuthority(publicBundle, password),
	};
}

/** A local certificate authority as the fake crypto creates it. */
export function fakeAuthority(
	scope: DeviceAccountScope,
	row: {
		authorityId: string;
		label: string;
		issuingNotAfter: number;
		rootNotAfter: number;
	},
	password = FAKE_PASSWORD,
): LocalCertificateAuthority {
	const publicBundle: CertificateAuthorityPublic = {
		account_binding: accountStorageKey(scope),
		authority_id: row.authorityId,
		label: row.label,
		dns_suffixes: [],
		ip_addresses: [],
		root_certificate_pem: PEM(`root:${row.authorityId}`),
		issuer_certificate_pem: PEM(`issuer:${row.authorityId}`),
		sha256_fingerprint: [...fakeBytes(`authority:${row.authorityId}`)]
			.map((byte) => byte.toString(16).padStart(2, "0"))
			.join(""),
		not_before: row.rootNotAfter - 10 * 365 * 86_400,
		not_after: row.rootNotAfter,
		issuer_not_after: row.issuingNotAfter,
	};
	return {
		public_bundle: publicBundle,
		vault: Uint8Array.from(sealAuthority(publicBundle, utf8(password))),
	};
}

function authorityFunctions(): Pick<
	DeviceCrypto,
	| "createCertificateAuthorityVault"
	| "inspectCertificateAuthorityVault"
	| "inspectCertificateAuthorityBackup"
	| "rewrapCertificateAuthorityVault"
	| "renewCertificateAuthorityVault"
	| "signServiceCertificate"
	| "signDeviceCertificateIssuer"
> {
	const DAY = 86_400;
	const sign: DeviceCrypto["signServiceCertificate"] = (
		accountBinding,
		authorityId,
		password,
		vault,
		request,
		now,
	) => {
		openAuthority(accountBinding, authorityId, password, vault);
		return {
			certificate_chain_pem: PEM(`leaf:${request.csr_pem}`),
			not_before: now,
			not_after: now + request.validity_days * DAY,
		};
	};
	return {
		createCertificateAuthorityVault: (spec, password, now) =>
			authorityEnvelope(
				{
					account_binding: spec.account_binding,
					authority_id: spec.authority_id,
					label: spec.label,
					dns_suffixes: spec.dns_suffixes,
					ip_addresses: spec.ip_addresses,
					root_certificate_pem: PEM(`root:${spec.authority_id}`),
					issuer_certificate_pem: PEM(`issuer:${spec.authority_id}`),
					sha256_fingerprint: [...fakeBytes(`authority:${spec.authority_id}`)]
						.map((byte) => byte.toString(16).padStart(2, "0"))
						.join(""),
					not_before: now,
					not_after: now + spec.validity_days * DAY,
					issuer_not_after: now + Math.min(spec.validity_days, 365) * DAY,
				},
				password,
			),
		inspectCertificateAuthorityVault: (binding, id, password, vault) =>
			openAuthority(binding, id, password, vault).public_bundle,
		inspectCertificateAuthorityBackup: (binding, id, password, vault) =>
			openAuthority(binding, id, password, vault).public_bundle,
		rewrapCertificateAuthorityVault: (binding, id, current, next, vault) =>
			authorityEnvelope(
				openAuthority(binding, id, current, vault).public_bundle,
				next,
			),
		renewCertificateAuthorityVault: (
			binding,
			id,
			password,
			vault,
			_root,
			now,
		) =>
			authorityEnvelope(
				{
					...openAuthority(binding, id, password, vault).public_bundle,
					issuer_not_after: now + 365 * DAY,
				},
				password,
			),
		signServiceCertificate: sign,
		signDeviceCertificateIssuer: sign,
	};
}

function ownerSignature<T extends { device_id: string }>(
	payload: T,
	password: Uint8Array,
	invitationVault: Uint8Array,
): string {
	const invitation = openVault(
		invitationVault,
		"invitation",
		payload.device_id,
		password,
	);
	return fakeCompact(payload, invitation.public_key);
}

/** A `DeviceCrypto` whose vaults, signatures and sealed data only this fake and the fake hub read. */
export function fakeDeviceCrypto(options: FakeCryptoOptions = {}): FakeCrypto {
	const controllers: FakeController[] = [];
	let loads = 0;
	const onboardingVaults = (password: Uint8Array) => {
		const deviceId = `onboarding-${crypto.randomUUID()}`;
		const controller: ControllerPublic = {
			device_id: deviceId,
			endpoint_id: "endpoint-1",
			controller_key: fakeKey(`controller:${deviceId}`),
			archive_key: Array.from(fakeBytes(`archive:${deviceId}`)),
			telemetry_member: {
				endpoint_id: "endpoint-1",
				signing_key: fakeKey(`member:${deviceId}`),
			},
		};
		const publicKey = fakeKey(`invitation:${deviceId}`);
		const check = fakePasswordCheck(deviceId, password);
		return {
			controller: {
				public_bundle: controller,
				vault: Array.from(
					sealFakeVault({
						fake: "controller",
						device_id: deviceId,
						check,
						controller,
					}),
				),
			},
			invitation: {
				public_key: publicKey,
				vault: Array.from(
					sealFakeVault({
						fake: "invitation",
						device_id: deviceId,
						check,
						public_key: publicKey,
					}),
				),
			},
		};
	};
	return {
		controllers,
		get loads() {
			return loads;
		},
		default: async () => {
			loads += 1;
		},
		...authorityFunctions(),
		unlockControllerVault: (deviceId, password, ciphertext) => {
			const controller = createController(
				openVault(ciphertext, "controller", deviceId, password),
				options,
			);
			controllers.push(controller);
			return controller;
		},
		verifyDeviceReceipt: (receipt, expectedManifest, controllerKey) => {
			if (receipt.manifest_jws !== expectedManifest)
				throw new Error("The receipt names another onboarding manifest.");
			const manifest = readFakeCompact<OnboardingManifest>(
				receipt.manifest_jws,
			);
			if (
				manifest.device_id !== receipt.device_id ||
				manifest.controller_key.x !== controllerKey.x
			)
				throw new Error("The receipt was not approved by this controller.");
			return manifest;
		},
		createOnboardingVaults: onboardingVaults,
		createControllerVault: (deviceId, password) => {
			const controller: ControllerPublic = {
				device_id: deviceId,
				endpoint_id: "endpoint-1",
				controller_key: fakeKey(`controller:${crypto.randomUUID()}`),
				archive_key: Array.from(fakeBytes(`archive:${crypto.randomUUID()}`)),
				telemetry_member: {
					endpoint_id: "endpoint-1",
					signing_key: fakeKey(`member:${crypto.randomUUID()}`),
				},
			};
			return {
				public_bundle: controller,
				vault: Array.from(
					sealFakeVault({
						fake: "controller",
						device_id: deviceId,
						check: fakePasswordCheck(deviceId, password),
						controller,
					}),
				),
			};
		},
		createInvitationVault: (deviceId, password) => {
			const publicKey = fakeKey(`invitation:${crypto.randomUUID()}`);
			return {
				public_key: publicKey,
				vault: Array.from(
					sealFakeVault({
						fake: "invitation",
						device_id: deviceId,
						check: fakePasswordCheck(deviceId, password),
						public_key: publicKey,
					}),
				),
			};
		},
		createBootstrapKey: () => ({
			public_key: fakeKey(`bootstrap:${crypto.randomUUID()}`),
			secret_base64: base64url(
				fakeBytes(`bootstrap-secret:${crypto.randomUUID()}`),
			),
		}),
		rewrapControllerVaults: (
			deviceId,
			currentPassword,
			newPassword,
			controllerVault,
			invitationVault,
		) => {
			const controller = openVault(
				controllerVault,
				"controller",
				deviceId,
				currentPassword,
			);
			const invitation = invitationVault.length
				? openVault(invitationVault, "invitation", deviceId, currentPassword)
				: undefined;
			return {
				controller: {
					public_bundle: controller.controller as ControllerPublic,
					vault: freshVault(controller, deviceId, newPassword),
				},
				invitation: invitation
					? {
							public_key: invitation.public_key as Ed25519PublicKey,
							vault: freshVault(invitation, deviceId, newPassword),
						}
					: null,
			};
		},
		sealAccountRecovery: sealFakeRecovery,
		openAccountRecovery: openFakeRecovery,
		signManagementPolicy: ownerSignature,
		signTelemetryRoster: ownerSignature,
		signArchiveRoster: ownerSignature,
		verifyManagementPolicy: (compact, ownerKey) =>
			readFakeCompact<ManagementPolicy>(compact, ownerKey),
		verifyArchiveRosterHead: (compact, ownerKey) =>
			readFakeCompact<ArchiveRoster>(compact, ownerKey),
		verifyHistoricalTelemetryRoster: (compact, ownerKey) =>
			readFakeCompact<TelemetryRoster>(compact, ownerKey),
		verifyTelemetryRoster: (compact, ownerKey, now) => {
			const roster = readFakeCompact<TelemetryRoster>(compact, ownerKey);
			if (roster.expires_at <= now)
				throw new Error("The reader list has expired.");
			return roster;
		},
	};
}

/* Seeding what this computer holds. */

const itemKey = (scopeKey: string, ...ids: string[]) =>
	JSON.stringify([scopeKey, ...ids]);

function store(browser: FakeBrowser, name: string): Rows {
	let rows = browser.stores.get(name);
	if (!rows) {
		rows = new Map();
		browser.stores.set(name, rows);
	}
	return rows;
}

function seededVault(
	hub: FakeHub,
	seed: DeviceSeed,
	summary: DeviceSeed["local"]["vaults"][number],
): LocalDeviceVault {
	const { deviceId } = summary;
	if (hub.rows.has(deviceId))
		return hub.localVault(deviceId, {
			restored: summary.requiresFreshEndpoint,
		});
	// Keys for a device the hub does not list: an access request still waiting for its owner.
	const request = seed.accessRequests?.find((row) => row.deviceId === deviceId);
	const owner = summary.role === "owner";
	return hub.localVault(deviceId, {
		ownerId: owner ? hub.me : (request?.ownerId ?? "usr_unlisted_owner"),
		name: request?.deviceName,
		grantId: owner ? undefined : summary.grantId,
		restored: summary.requiresFreshEndpoint,
	});
}

const toMillis = (time: number) =>
	time < 100_000_000_000 ? time * 1000 : time;

async function recoveryState(
	scope: DeviceAccountScope,
	vault: LocalDeviceVault | undefined,
	backup: DeviceSeed["local"]["backups"][string],
): Promise<AccountRecoveryState> {
	const digest = vault ? await backupSourceDigest(scope, vault) : "no-vault";
	const sourceDigest =
		backup.sourceDigestChanged === undefined
			? undefined
			: backup.sourceDigestChanged
				? fakeDigest(`changed:${digest}`)
				: digest;
	const flag = backup.passwordChangedSinceBackup
		? { passwordChangedSinceBackup: true }
		: {};
	if (!backup.pending)
		return {
			revision: backup.localRevision,
			...(sourceDigest ? { sourceDigest } : {}),
			...flag,
		};
	const publicKey =
		vault?.controllerPublic.controller_key ?? fakeKey("no-vault");
	return {
		revision: Math.max(0, backup.localRevision - 1),
		...(sourceDigest ? { sourceDigest } : {}),
		pending: {
			sourceDigest: digest,
			request: {
				public_key: publicKey,
				ciphertext: base64url(fakeBytes(`pending:${vault?.deviceId}`)),
				revision: Math.max(1, backup.localRevision),
				proof_jws: fakeCompact({ pending: vault?.deviceId }, publicKey),
			},
		},
		...flag,
	};
}

async function seedLocal(
	browser: FakeBrowser,
	api: FakeDeviceApi,
	seed: DeviceSeed,
) {
	const { scope, hub } = api;
	const scopeKey = accountStorageKey(scope);
	const vaults = store(browser, "vaults");
	const held = new Map<string, LocalDeviceVault>();
	for (const summary of seed.local.vaults) {
		const vault = seededVault(hub, seed, summary);
		held.set(summary.deviceId, vault);
		vaults.set(itemKey(scopeKey, summary.deviceId), vault);
		if (summary.identityPinnedAt === undefined) continue;
		vaults.set(
			itemKey(
				scopeKey,
				summary.deviceId,
				"identity",
				hub.manifest(summary.deviceId).enrollment_id,
			),
			{
				identity: deviceIdentityKey(fakeKeys.identity(summary.deviceId)),
				pinnedAt: toMillis(summary.identityPinnedAt),
			},
		);
	}
	const recovery = store(browser, "recovery");
	for (const [deviceId, backup] of Object.entries(seed.local.backups))
		recovery.set(
			itemKey(scopeKey, deviceId),
			await recoveryState(scope, held.get(deviceId), backup),
		);
	const authorities = store(browser, "authorities");
	for (const row of seed.local.authorities)
		authorities.set(
			itemKey(scopeKey, row.authorityId),
			fakeAuthority(scope, row, hub.password),
		);
}

function seedMemory(
	api: FakeDeviceApi,
	seed: DeviceSeed,
	facts: Pick<Storage, "setItem">,
) {
	const scopeKey = accountStorageKey(api.scope);
	globalThis.localStorage?.setItem(
		`${ACTIVITY_STORAGE_PREFIX}${scopeKey}`,
		JSON.stringify({ v: 1, items: seed.activity, runs: [] }),
	);
	facts.setItem(
		`${FACTS_PREFIX}${scopeKey}`,
		JSON.stringify({
			agents: seed.agentLastRead ?? {},
			requests: seed.accessRequests ?? [],
		}),
	);
}

/* The fake workspace. */

export interface FakeWorkspaceOptions
	extends Omit<FakeDeviceApiOptions, "seed" | "now"> {
	/** A fake api built beforehand; its seed is used. */
	api?: FakeDeviceApi;
	platform?: "desktop" | "web";
	/** Key sessions open at the start: the seed's unlocked ones (default), none, or these devices. */
	unlock?: "seed" | "none" | string[];
	/** Epoch milliseconds the clock starts at; defaults to the seed's `now`. */
	now?: number;
	/** `false` leaves `Date.now()` alone and moves only the injected clock. */
	freezeDate?: boolean;
	/** `false`: a crypto bundle without the held-signer methods (owner signatures ask for the password). */
	heldSigner?: boolean;
	/**
	 * `false`: nothing is reported for the seed's open devices beyond what the
	 * workspace reads by itself. By default the seed's rollouts, history readers,
	 * certificate requests and renewals and endpoint facts count as read, as if
	 * their tabs had been opened.
	 */
	viewFacts?: boolean;
	persistence?: FakePersistence;
	queryClient?: QueryClient;
	/** Extra workspace seams, merged over the fake's. */
	workspace?: DeviceWorkspaceOptions;
}

export interface FakeWorkspace {
	readonly seed: DeviceSeed;
	readonly api: FakeDeviceApi;
	readonly hub: FakeHub;
	readonly crypto: FakeCrypto;
	readonly clock: FakeClock;
	readonly browser: FakeBrowser;
	readonly queryClient: QueryClient;
	readonly scope: DeviceAccountScope;
	readonly profile: IProfile;
	readonly password: string;
	readonly deps: WorkspaceDeps;
	/** The seams the workspace was built with, for a test that goes through the registry itself. */
	readonly workspaceOptions: DeviceWorkspaceOptions;
	readonly workspace: DeviceWorkspaceRuntime;
	agent(deviceId: string): FakeAgent;
	/** Unlock with the seed password and wait until the encrypted status (and, with `connectLive`, the live read) arrived. */
	unlock(deviceId: string, options?: UnlockOptions): Promise<void>;
	/** Another window holds the device's key lock until the returned function is called. */
	holdElsewhere(deviceId: string): () => void;
	/** Let pending reads, timers of 0 ms and store updates finish. */
	settle(): Promise<void>;
	/** Lock everything and put every global back. */
	dispose(): Promise<void>;
}

const tick = () => new Promise<void>((resolve) => setTimeout(resolve, 0));

async function waitFor(done: () => boolean, timeoutMs = 2_000): Promise<void> {
	const started = performance.now();
	while (!done()) {
		if (performance.now() - started > timeoutMs)
			throw new Error("The fake workspace did not settle in time.");
		await tick();
	}
}

function unlockTargets(
	seed: DeviceSeed,
	option: FakeWorkspaceOptions["unlock"],
) {
	if (option === "none") return [];
	if (Array.isArray(option)) return option;
	return seed.keys
		.filter((session) => session.state === "unlocked")
		.map((session) => session.deviceId);
}

export async function createFakeWorkspace(
	seed?: DeviceSeed,
	options: FakeWorkspaceOptions = {},
): Promise<FakeWorkspace> {
	const fleet = options.api?.seed ?? seed ?? fakeDeviceApi().seed;
	const clock = createFakeClock(
		options.now ?? fleet.now * 1000,
		options.freezeDate !== false,
	);
	const api =
		options.api ?? fakeDeviceApi({ ...options, seed: fleet, now: clock.now });
	api.hub.now = () => Math.floor(clock.now() / 1000);
	const browser = installFakeBrowser({
		persistence: options.persistence,
		WebSocket: api.WebSocket,
		fetch: api.hubFetch,
	});
	if (fleet.latestRelease && !api.hub.release)
		await api.hub.publishRelease(fleet.latestRelease);
	const crypto = fakeDeviceCrypto({ heldSigner: options.heldSigner });
	const facts = new Map<string, string>();
	const factsStorage = {
		getItem: (key: string) => facts.get(key) ?? null,
		setItem: (key: string, value: string) => {
			facts.set(key, value);
		},
	};
	await seedLocal(browser, api, fleet);
	seedMemory(api, fleet, factsStorage);
	const queryClient =
		options.queryClient ??
		new QueryClient({ defaultOptions: { queries: { retry: false } } });
	const deps: WorkspaceDeps = {
		api,
		profile: api.profile,
		scope: api.scope,
		queryClient,
		crypto: async () => crypto,
		platform: options.platform ?? fleet.local.platform,
		now: clock.now,
	};
	const workspaceOptions: DeviceWorkspaceOptions = {
		fetch: api.hubFetch,
		storage: factsStorage,
		visible: () => true,
		...options.workspace,
	};
	const workspace = createDeviceWorkspace(deps, workspaceOptions);
	// A provider without overrides (the registry path) binds to this workspace too.
	registerDeviceWorkspace(workspace);
	await workspace.local.reload();

	const settle = async () => {
		for (let round = 0; round < 3; round++) await tick();
	};
	let disposed = false;
	const close = async () => {
		if (openDeviceWorkspace(workspace.scopeKey) !== workspace) {
			await workspace.dispose();
			return;
		}
		await disposeDeviceWorkspace(workspace.scopeKey);
		// The registry reports a disposal as a sign-out; that notice is not for the next test.
		if (lastWorkspaceSwitch()?.from === api.scope) dismissWorkspaceSwitch();
	};
	const unlock = async (
		deviceId: string,
		unlockOptions: UnlockOptions = {},
	) => {
		await workspace.keys.unlock(deviceId, api.hub.password, unlockOptions);
		if (unlockOptions.connectLive)
			await waitFor(
				() =>
					workspace.live.inspection(deviceId) !== undefined ||
					!["connecting", "live"].includes(workspace.live.state(deviceId).kind),
			);
		await settle();
	};
	/** What the device's tabs would have read by now, as the seed states it. */
	const reportViewFacts = (deviceId: string) => {
		const live = fleet.live[deviceId];
		if (!live || options.viewFacts === false) return;
		const { state: _state, inspection: _inspection, ...read } = live;
		const {
			offlineQueues: _queues,
			certificates: _certificates,
			...facts
		} = read;
		if (Object.keys(facts).length) workspace.facts.record(deviceId, facts);
	};
	const fake: FakeWorkspace = {
		seed: fleet,
		api,
		hub: api.hub,
		crypto,
		clock,
		browser,
		queryClient,
		scope: api.scope,
		profile: api.profile,
		password: api.hub.password,
		deps,
		workspaceOptions,
		workspace,
		agent: (deviceId) => api.agent(deviceId),
		unlock,
		holdElsewhere: (deviceId) =>
			browser.locks.takeElsewhere(
				itemKey(accountStorageKey(api.scope), deviceId, "unlock"),
			),
		settle,
		async dispose() {
			if (disposed) return;
			disposed = true;
			await close();
			queryClient.clear();
			await settle();
			browser.restore();
			clock.restore();
		},
	};
	try {
		for (const deviceId of unlockTargets(fleet, options.unlock)) {
			await unlock(deviceId, {
				connectLive: fleet.live[deviceId]?.state.kind === "live",
			});
			reportViewFacts(deviceId);
		}
	} catch (error) {
		await fake.dispose();
		throw error;
	}
	return fake;
}
