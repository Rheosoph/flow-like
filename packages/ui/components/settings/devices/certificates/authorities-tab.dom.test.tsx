import { afterAll, afterEach, describe, expect, test } from "bun:test";
import { sampleFleet } from "../../../../lib/device-management/model/__fixtures__/sample-fleet";
import type { CertificatesRoute } from "../../../../lib/device-management/model/types";
import {
	allByRole,
	byRole,
	click,
	dropFiles,
	inPortal,
	installDom,
	queryByRole,
	typeInto,
} from "../testing/dom-harness";
import type { MountDevicesOptions } from "../testing/mount-devices";

const dom = installDom();
const { cleanupDevices, mountDevices, preloadDevices } = await import(
	"../testing/mount-devices"
);
await preloadDevices();
const { readCertificateAuthorities } = await import(
	"../../../../lib/device-management/storage"
);
const { useDevicesRoute } = await import("../routing/use-devices-route");
const { CertificatesScreen } = await import("./certificates-screen");

interface Download {
	name: string;
	text: string;
}

/** What a download hands to the browser: recorded instead of saved. */
function recordDownloads() {
	const view = window as unknown as typeof globalThis;
	const blobs = new Map<string, Blob>();
	const pending: Promise<void>[] = [];
	const downloads: Download[] = [];
	const createObjectURL = URL.createObjectURL;
	const revokeObjectURL = URL.revokeObjectURL;
	const anchorClick = view.HTMLAnchorElement.prototype.click;
	let next = 0;
	URL.createObjectURL = (blob: Blob) => {
		const url = `blob:test-${next++}`;
		blobs.set(url, blob);
		return url;
	};
	URL.revokeObjectURL = () => undefined;
	view.HTMLAnchorElement.prototype.click = function click(
		this: HTMLAnchorElement,
	) {
		const blob = blobs.get(this.getAttribute("href") ?? "");
		const name = this.download;
		if (blob && name)
			pending.push(
				blob.text().then((text) => {
					downloads.push({ name, text });
				}),
			);
	};
	return {
		async all() {
			await Promise.all(pending);
			return downloads;
		},
		restore() {
			URL.createObjectURL = createObjectURL;
			URL.revokeObjectURL = revokeObjectURL;
			view.HTMLAnchorElement.prototype.click = anchorClick;
		},
	};
}

let recorder = recordDownloads();

afterEach(async () => {
	await cleanupDevices();
	recorder.restore();
	recorder = recordDownloads();
	await dom.cleanup();
});
afterAll(() => {
	recorder.restore();
	dom.restore();
});

const ACCOUNT = { kind: "account" } as const;
const AUTHORITIES: CertificatesRoute = {
	screen: "certificates",
	tab: "authorities",
};
const LABEL = "Rheosoph Internal";
const NEW_PASSWORD = "a strong authority password";
const DAY_S = 86_400;
/** R3: wire values, condition keys and gate codes never reach the screen. */
const MACHINE_WORDS =
	/[a-z]+_[a-z_]+|\bG\d{1,2}\b|\bD\d{1,2}\b|lets_encrypt|vault\b/;

function mount(options: MountDevicesOptions = {}) {
	return mountDevices(
		<CertificatesScreen route={AUTHORITIES} scope={ACCOUNT} />,
		{ search: "view=certificates&tab=authorities", ...options },
	);
}

type View = Awaited<ReturnType<typeof mount>>;

function noAuthorities(): MountDevicesOptions {
	const seed = sampleFleet();
	seed.local = { ...seed.local, authorities: [] };
	return { seed };
}

/** The sample authority with its signing key expiring `seconds` from now (negative: already expired). */
function withSigningKeyUntil(seconds: number): MountDevicesOptions {
	const seed = sampleFleet();
	seed.local = {
		...seed.local,
		authorities: seed.local.authorities.map((authority) => ({
			...authority,
			issuingNotAfter: seed.now + seconds,
		})),
	};
	return { seed };
}

/** The screen on the area's own route, so tabs and links really switch. */
function Routed() {
	const { route, scope } = useDevicesRoute();
	return <CertificatesScreen route={route} scope={scope} />;
}

const stored = (view: View) => readCertificateAuthorities(view.fake.scope);

function field(name: string | RegExp, root?: ParentNode): HTMLInputElement {
	return byRole("textbox", name, root) as HTMLInputElement;
}

function passwordValues(root: ParentNode): string[] {
	return Array.from(
		root.querySelectorAll<HTMLInputElement>("[data-secret] input"),
	).map((input) => input.value);
}

/** The seeded authority's backup as the wizard would have written it. */
async function backupFile(view: View, name = "7e2d9c41-authority.json") {
	const [authority] = await stored(view);
	if (!authority) throw new Error("No authority is stored");
	const vault = Array.from(authority.vault);
	return new File(
		[
			JSON.stringify({
				version: 1,
				public_bundle: authority.public_bundle,
				vault,
				root_vault: vault,
			}),
		],
		name,
		{ type: "application/json" },
	);
}

async function dropBackup(view: View, file: File) {
	const zone = inPortal("dialog").querySelector("label[data-over], label[for]");
	const drop = Array.from(
		inPortal("dialog").querySelectorAll<HTMLElement>("label"),
	).find((label) => label.querySelector('input[type="file"]'));
	await dropFiles((drop ?? zone) as HTMLElement, [file]);
	await view.settle();
}

/** Names → lifetime → password, up to the point where the authority exists only in memory. */
async function createUntilBackup(view: View) {
	const { container } = view;
	await click(byRole("button", "Create authority…", container));
	await typeInto(field("Name", container), "Rheosoph Lab");
	await typeInto(field("DNS suffix 1", container), "lab.example.com");
	await click(byRole("button", /^Continue/, container));
	await click(byRole("button", /^Continue/, container));
	await typeInto(field("Authority password", container), NEW_PASSWORD);
	await typeInto(field("Type the password again", container), NEW_PASSWORD);
	await click(byRole("button", /^Create authority/, container));
	await view.settle();
}

describe("Create authority (inherits device-certificate-authorities 1–2)", () => {
	test("the issuing key is persisted only after the user acknowledges the offline root backup", async () => {
		const view = await mount(noAuthorities());
		const { container } = view;
		expect(container.textContent).toContain(
			"No organisation authority on this computer",
		);
		expect(container.querySelectorAll("[data-dv-primary]")).toHaveLength(1);
		await createUntilBackup(view);
		expect(container.textContent).toContain(
			"This backup is the only copy of the root key.",
		);
		expect(await stored(view)).toEqual([]);

		const use = byRole("button", /^Use this authority/, container);
		expect(use.getAttribute("aria-disabled")).toBe("true");
		expect(container.textContent).toContain(
			"Download the backup and tick the box first.",
		);
		const saved = byRole(
			"checkbox",
			"I saved the backup and its password",
			container,
		);
		expect(saved.hasAttribute("disabled")).toBe(true);
		await click(use);
		await view.settle();
		expect(await stored(view)).toEqual([]);

		await click(
			byRole("button", "Download encrypted authority backup", container),
		);
		const [download] = await recorder.all();
		expect(download?.name).toMatch(/^[0-9a-f]{8}-authority\.json$/);
		expect(JSON.parse(download?.text ?? "{}")).toHaveProperty("root_vault");
		expect(container.textContent).toContain(
			"Tick the box once the backup and password are stored.",
		);
		await click(
			byRole("checkbox", "I saved the backup and its password", container),
		);
		await click(byRole("button", /^Use this authority/, container));
		await view.settle();

		const authorities = await stored(view);
		expect(authorities).toHaveLength(1);
		expect(authorities[0]).not.toHaveProperty("root_vault");
		expect(authorities[0]?.public_bundle.label).toBe("Rheosoph Lab");
		expect(authorities[0]?.public_bundle.dns_suffixes).toEqual([
			"lab.example.com",
		]);
		expect(passwordValues(container)).toEqual([]);
		expect(container.textContent).toContain(
			"Rheosoph Lab is ready on this computer.",
		);
		expect(
			container.querySelectorAll("[data-dv-primary]").length,
		).toBeLessThanOrEqual(1);

		await click(byRole("button", "Back to authorities", container));
		expect(container.textContent).toContain("is ready to sign");
		expect(byRole("heading", "Rheosoph Lab", container)).toBeTruthy();
	});

	test("failed authority creation clears passwords and hides cryptography exception contents", async () => {
		const view = await mount(noAuthorities());
		const { container } = view;
		view.fake.crypto.createCertificateAuthorityVault = () => {
			throw new Error("private-key-material-must-not-be-rendered");
		};
		await createUntilBackup(view);
		expect(await stored(view)).toEqual([]);
		expect(passwordValues(container)).toEqual(["", ""]);
		expect(container.textContent).toContain(
			"The authority couldn't be created.",
		);
		expect(container.textContent).not.toContain("private-key-material");
	});

	test("names an authority can't sign for are refused before anything is created", async () => {
		const view = await mount(noAuthorities());
		const { container } = view;
		await click(byRole("button", "Create authority…", container));
		await click(byRole("button", /^Continue/, container));
		expect(container.textContent).toContain(
			"Give the authority a name, for example Rheosoph Lab.",
		);
		expect(container.textContent).toContain(
			"Add at least one DNS suffix or IP address.",
		);
		await typeInto(field("Name", container), "Rheosoph Lab");
		await typeInto(field("DNS suffix 1", container), "*.lab.example.com");
		await typeInto(field("IP address 1", container), "10.0.4.0/24");
		await click(byRole("button", /^Continue/, container));
		expect(container.textContent).toContain(
			"DNS suffix 1: Wildcards aren't allowed.",
		);
		expect(container.textContent).toContain(
			"IP address 1: Enter one IP address, like 10.0.0.20.",
		);
		expect(container.textContent).toContain(
			"What may this authority sign for?",
		);
	});

	test("leaving before the backup is saved warns that the root key is lost, and discarding stores nothing", async () => {
		const view = await mount(noAuthorities());
		const { container } = view;
		await createUntilBackup(view);
		const navigations = view.navigations.length;
		// Another tab is a way out too: the page stays and asks first.
		await click(byRole("tab", /Reminders/, container));
		expect(view.navigations).toHaveLength(navigations);
		const confirm = byRole("region", "Discard the new authority?", container);
		expect(confirm.textContent).toContain(
			"The root key exists only in this backup. If you close now, it's lost.",
		);
		expect(confirm.textContent).toContain("No, this is permanent.");
		await click(byRole("button", "Cancel", confirm));
		expect(
			queryByRole("region", "Discard the new authority?", container),
		).toBeNull();

		await click(byRole("button", "Discard authority…", container));
		await click(
			byRole(
				"button",
				"Discard authority",
				byRole("region", "Discard the new authority?", container),
			),
		);
		await view.settle();
		expect(container.textContent).toContain(
			"Nothing was saved on this computer.",
		);
		expect(container.textContent).toContain(
			"No organisation authority on this computer",
		);
		expect(await stored(view)).toEqual([]);
	});
});

describe("An authority on this computer", () => {
	test("shows its names, lifetime, where its keys live and what it signed, with one source per block", async () => {
		const view = await mount();
		const { container } = view;
		const card = container.querySelector("#authority-7e2d9c41");
		expect(card?.textContent).toContain(LABEL);
		expect(card?.textContent).toContain("Active");
		expect(card?.textContent).toContain("Root fingerprint");
		expect(card?.textContent).toContain(
			"Only in your backup file. Not on this computer, never uploaded.",
		);
		expect(card?.textContent).toContain("internal-mqtt");
		expect(card?.querySelector("header [data-stamp]")?.textContent).toContain(
			"stored on this computer",
		);
		expect(card?.textContent).toContain(
			"Other computers, browsers, hubs and profiles don't see it",
		);
		expect(container.querySelectorAll("[data-dv-primary]")).toHaveLength(0);
		expect(container.textContent).not.toMatch(MACHINE_WORDS);
	});

	test("a signing key that stops signing soon is called out on the card, the tab and the headline", async () => {
		const view = await mount(withSigningKeyUntil(11 * DAY_S));
		const { container } = view;
		const card = container.querySelector("#authority-7e2d9c41");
		expect(card?.textContent).toContain("Stops signing in 11 d");
		expect(card?.querySelector("[data-tone=warning]")).not.toBeNull();
		expect(card?.textContent).toContain(`${LABEL} stops signing on`);
		expect(
			allByRole("button", "Renew signing key…", card as HTMLElement),
		).toHaveLength(2);
		expect(container.querySelector("[data-headline]")?.textContent).toContain(
			`Your authority ${LABEL} stops signing on`,
		);
		expect(
			byRole("tab", /Organisation authorities/, container).querySelector(
				"[data-count-tone=warning]",
			),
		).not.toBeNull();
	});

	test("an authority whose signing key expired can't sign: the card says so, and fixing a renewal is gated and offers the restore", async () => {
		const view = await mountDevices(<Routed />, {
			search: "view=certificates",
			...withSigningKeyUntil(-3600),
		});
		const { container } = view;
		expect(container.querySelector("[data-headline]")?.textContent).toContain(
			"Fixing the renewal of internal-mqtt needs an organisation authority that can sign, and none is on this computer.",
		);
		const row = container.querySelector(
			'tr[data-certificate="93bcc1ef-5f49-4bb3-b90c-7b052822bf02"]',
		) as HTMLElement;
		const fix = byRole("button", "Fix renewal", row);
		expect(fix.getAttribute("aria-disabled")).toBe("true");
		expect(row.textContent).toContain(
			"Needs an organisation authority that can sign on this computer.",
		);
		const navigations = view.navigations.length;
		await click(fix);
		expect(view.navigations).toHaveLength(navigations);
		await click(byRole("button", "Restore one from backup…", row));
		await view.settle();
		expect(view.navigations.at(-1)?.href).toContain("tab=authorities");
		expect(inPortal("dialog").textContent).toContain(
			"Restore an authority from its backup",
		);
		const card = container.querySelector("#authority-7e2d9c41");
		expect(card?.textContent).toContain("Signing key expired");
		expect(card?.textContent).toContain(
			`${LABEL} can't sign any more: its signing key expired`,
		);
	});

	test("downloads the root and the signing certificate", async () => {
		const view = await mount();
		const { container } = view;
		const [authority] = await stored(view);
		await click(byRole("button", "Download root certificate", container));
		await click(byRole("button", "Download signing certificate", container));
		const downloads = await recorder.all();
		expect(downloads.map((download) => download.name)).toEqual([
			"7e2d9c41-root.crt",
			"7e2d9c41-signing.crt",
		]);
		expect(downloads[0]?.text).toBe(
			authority?.public_bundle.root_certificate_pem ?? "",
		);
		expect(downloads[1]?.text).toBe(
			authority?.public_bundle.issuer_certificate_pem ?? "",
		);
		expect(container.textContent).toContain(
			"Some clients need it next to the root to complete the chain.",
		);
	});

	test("testing the authority password says whether it opens the key, clears the field and changes nothing", async () => {
		const view = await mount();
		const before = await stored(view);
		await click(byRole("button", "Test authority password…", view.container));
		let sheet = inPortal("dialog");
		await typeInto(field("Authority password", sheet), "not the password");
		await click(byRole("button", "Test password", sheet));
		await view.settle();
		sheet = inPortal("dialog");
		expect(sheet.textContent).toContain(
			`That password doesn't open the signing key of ${LABEL}.`,
		);
		expect(passwordValues(sheet)).toEqual([""]);
		await typeInto(field("Authority password", sheet), view.fake.password);
		await click(byRole("button", "Test password", sheet));
		await view.settle();
		expect(inPortal("dialog").textContent).toContain(
			`That password opens the signing key of ${LABEL}.`,
		);
		expect(passwordValues(inPortal("dialog"))).toEqual([""]);
		expect(await stored(view)).toEqual(before);
	});

	test("changing the authority password needs the backup, clears the fields on a wrong password and stores the key only after the new backup is saved", async () => {
		const view = await mount();
		const [before] = await stored(view);
		const file = await backupFile(view);
		await click(byRole("button", "Change authority password…", view.container));
		let sheet = inPortal("dialog");
		expect(sheet.textContent).toContain("Before this runs");
		await dropBackup(view, file);
		sheet = inPortal("dialog");
		await typeInto(field("Current authority password", sheet), "wrong one");
		await typeInto(field("New authority password", sheet), NEW_PASSWORD);
		await typeInto(field("Type the new password again", sheet), NEW_PASSWORD);
		await click(byRole("button", "Change password", sheet));
		await view.settle();
		sheet = inPortal("dialog");
		expect(sheet.textContent).toContain("That isn't the current password");
		expect(passwordValues(sheet)).toEqual(["", "", ""]);
		expect((await stored(view))[0]?.vault).toEqual(before?.vault as Uint8Array);

		await typeInto(
			field("Current authority password", sheet),
			view.fake.password,
		);
		await typeInto(field("New authority password", sheet), NEW_PASSWORD);
		await typeInto(field("Type the new password again", sheet), NEW_PASSWORD);
		await click(byRole("button", "Change password", sheet));
		await view.settle();
		sheet = inPortal("dialog");
		expect(sheet.textContent).toContain(
			"Only the new backup opens with the new password.",
		);
		const use = byRole("button", "Use the new password", sheet);
		expect(use.hasAttribute("disabled")).toBe(true);
		expect((await stored(view))[0]?.vault).toEqual(before?.vault as Uint8Array);
		await click(byRole("button", "Download new authority backup", sheet));
		await click(
			byRole("checkbox", "I saved the new backup and its password", sheet),
		);
		await click(byRole("button", "Use the new password", inPortal("dialog")));
		await view.settle();
		expect((await stored(view))[0]?.vault).not.toEqual(
			before?.vault as Uint8Array,
		);
		expect(view.container.textContent).toContain(
			`Password of ${LABEL} changed at`,
		);

		await click(byRole("button", "Test authority password…", view.container));
		await typeInto(
			field("Authority password", inPortal("dialog")),
			NEW_PASSWORD,
		);
		await click(byRole("button", "Test password", inPortal("dialog")));
		await view.settle();
		expect(inPortal("dialog").textContent).toContain(
			`That password opens the signing key of ${LABEL}.`,
		);
	});

	test("renewing the signing key needs the backup and the password, and replaces the key only after the new backup is saved", async () => {
		const view = await mount();
		const [before] = await stored(view);
		const file = await backupFile(view);
		await click(byRole("button", "Renew signing key…", view.container));
		let sheet = inPortal("dialog");
		await click(byRole("button", "Renew signing key", sheet));
		expect(inPortal("dialog").textContent).toContain(
			"Choose the backup file first.",
		);
		await dropBackup(view, file);
		sheet = inPortal("dialog");
		await typeInto(
			field(`Authority password for ${LABEL}`, sheet),
			"wrong password",
		);
		await click(byRole("button", "Renew signing key", sheet));
		await view.settle();
		sheet = inPortal("dialog");
		expect(sheet.textContent).toContain(
			`That file and password don't open the backup of ${LABEL}. Nothing changed.`,
		);
		expect(passwordValues(sheet)).toEqual([""]);

		await typeInto(
			field(`Authority password for ${LABEL}`, sheet),
			view.fake.password,
		);
		await click(byRole("button", "Renew signing key", sheet));
		await view.settle();
		sheet = inPortal("dialog");
		expect(sheet.textContent).toContain(
			`Your old backup file no longer matches ${LABEL}.`,
		);
		expect((await stored(view))[0]?.public_bundle.issuer_not_after).toBe(
			before?.public_bundle.issuer_not_after as number,
		);
		await click(byRole("button", "Download new authority backup", sheet));
		await click(
			byRole("checkbox", "I saved the new backup and its password", sheet),
		);
		await click(
			byRole("button", "Use renewed signing key", inPortal("dialog")),
		);
		await view.settle();
		expect((await stored(view))[0]?.public_bundle.issuer_not_after).not.toBe(
			before?.public_bundle.issuer_not_after as number,
		);
		expect(view.container.textContent).toContain("Signing key renewed at");
		expect(queryByRole("dialog")).toBeNull();
	});

	test("removing the signing key asks for its typed label, and the backup restores it", async () => {
		const view = await mount();
		const { container } = view;
		const file = await backupFile(view);
		await click(
			byRole("button", "Remove signing key from this computer…", container),
		);
		const confirm = inPortal("alertdialog");
		expect(confirm.textContent).toContain(
			"You can't sign with it here until you restore it.",
		);
		const remove = byRole(
			"button",
			`Remove the signing key of ${LABEL}`,
			confirm,
		);
		expect(remove.getAttribute("aria-disabled")).toBe("true");
		await click(remove);
		await view.settle();
		expect(await stored(view)).toHaveLength(1);
		await typeInto(field(/./, confirm), LABEL);
		await click(
			byRole(
				"button",
				`Remove the signing key of ${LABEL}`,
				inPortal("alertdialog"),
			),
		);
		await view.settle();
		expect(await stored(view)).toEqual([]);
		expect(container.textContent).toContain("Not on this computer");
		expect(container.textContent).toContain(
			"Signing key removed from this computer at",
		);

		await click(byRole("button", "Restore from backup…", container));
		let sheet = inPortal("dialog");
		await dropBackup(view, file);
		sheet = inPortal("dialog");
		await typeInto(field("Authority password", sheet), "wrong password");
		await click(byRole("button", "Restore authority", sheet));
		await view.settle();
		sheet = inPortal("dialog");
		expect(sheet.textContent).toContain(
			"The backup couldn't be opened for this account and hub.",
		);
		expect(passwordValues(sheet)).toEqual([""]);
		expect(await stored(view)).toEqual([]);

		await typeInto(field("Authority password", sheet), view.fake.password);
		await click(byRole("button", "Restore authority", sheet));
		await view.settle();
		expect(await stored(view)).toHaveLength(1);
		expect(container.textContent).toContain(`Restored ${LABEL} at`);
		expect(
			container.querySelector("#authority-7e2d9c41")?.textContent,
		).toContain("Restored");
	});
});
