import { afterAll, expect, mock, test } from "bun:test";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { Window } from "happy-dom";
import { act } from "react";
import { ApiResponseError } from "../../../lib/api-error";
import type {
	Inspection,
	PlacementStatus,
} from "../../../lib/device-management/types";
import type {
	BillingGrant,
	ResourceGrant,
} from "../../../lib/device-resources";
import type { DeviceStatus } from "../../../lib/devices";

const window = new Window({ url: "https://app.example.com" });
Object.assign(window, { SyntaxError, TypeError, Error });
Object.assign(globalThis, {
	window,
	document: window.document,
	navigator: window.navigator,
	HTMLElement: window.HTMLElement,
	Element: window.Element,
	Node: window.Node,
	DocumentFragment: window.DocumentFragment,
	HTMLInputElement: window.HTMLInputElement,
	MutationObserver: window.MutationObserver,
	CustomEvent: window.CustomEvent,
	Event: window.Event,
	NodeFilter: window.NodeFilter,
	getComputedStyle: window.getComputedStyle.bind(window),
	requestAnimationFrame: (callback: FrameRequestCallback) =>
		setTimeout(() => callback(0), 0),
	cancelAnimationFrame: clearTimeout,
	IS_REACT_ACT_ENVIRONMENT: true,
});
let account = "owner-a";
let authenticated = true;
let enabled = true;
let rejectRevoke = true;
const calls: [string, string][] = [];
const statuses = new Map<string, DeviceStatus["status"]>();
let grants: ResourceGrant[] = [];
let billing: BillingGrant[] = [];
let resourceInstances: Record<string, unknown>[] | undefined;
let resourceError = false;
let loseGrantResponse = false;
let loseBillingResponse = false;
let loseRevokeResponse = false;
let replacementBilling = false;
const posts: { path: string; body: Record<string, unknown> }[] = [];
let latePost: (() => void) | undefined;
type ManagementDialogProps = {
	device: DeviceStatus;
	projectId?: string;
	onInspection?: (inspection: Inspection) => void;
	onClose: () => void;
};
let managementDialog: ManagementDialogProps | undefined;
const backend = {
	userState: { getProfile: async () => ({}) },
	apiState: {
		get: async (_profile: unknown, path: string) => {
			calls.push(["GET", path]);
			if (path.endsWith("/certificate-inventory"))
				return {
					revision: 1,
					updated_at: 100,
					certificates:
						account === "certificate-owner"
							? [
									{
										certificate_id: "00000000-0000-4000-8000-000000000001",
										revision: 1,
										fingerprint_sha256: "a".repeat(64),
										not_after: Math.floor(Date.now() / 1000) + 3600,
									},
								]
							: [],
				};
			if (path !== "devices") {
				if (resourceError)
					throw new ApiResponseError({
						status: 503,
						message: "Resource service unavailable.",
					});
				if (path.endsWith("/resource-grants")) return grants;
				if (path.endsWith("/billing-grants")) return billing;
				if (path.endsWith("/instances"))
					return (
						resourceInstances ??
						(grants.length
							? [
									{
										instance_id: "instance-1",
										device_id: `${account}-device`,
										grant_id: grants[0].grant_id,
										billing_grant_id: "billing-1",
										registered_at: 100,
										lease_expires_at: 1,
										registration_jws: "PUBLIC-SIGNED-RECEIPT",
									},
								]
							: [])
					);
			}
			return [
				{
					device_id: `${account}-device`,
					owner_id: account,
					name: `${account} server`,
					status: statuses.get(account) ?? "active",
					registered_at: 100,
					last_seen_at: null,
					auth_epoch: 1,
				},
			];
		},
		post: async (
			_profile: unknown,
			path: string,
			body: Record<string, unknown>,
		) => {
			posts.push({ path, body });
			if (path.endsWith("/billing")) {
				const value = {
					billing_grant_id: replacementBilling
						? "previous-billing"
						: "billing-1",
					grant_id: "grant-1",
					payer_id: account,
					authz_version: 1,
					limit_micros: replacementBilling
						? 5_000_000
						: (body.limit_micros as number),
					used_micros: 1,
					reserved_micros: 1_000_000,
					expires_at: body.expires_at as number,
					status: "active" as const,
				};
				billing = [value];
				if (loseBillingResponse)
					throw new ApiResponseError({
						status: 503,
						message: "Billing response was lost.",
					});
				return value;
			}
			const value = {
				...body,
				grant_id: "grant-1",
				device_id: `${account}-device`,
				delegating_user_id: account,
				authz_version: 1,
				status: "active",
			} as ResourceGrant;
			grants = [value];
			if (latePost)
				await new Promise<void>((resolve) => {
					latePost = resolve;
				});
			if (loseGrantResponse)
				throw new ApiResponseError({
					status: 503,
					message: "Resource response was lost.",
				});
			return value;
		},
		del: async (_profile: unknown, path: string) => {
			calls.push(["DELETE", path]);
			if (path.includes("/billing-grants/")) {
				billing = billing.map((row) => ({ ...row, status: "revoked" }));
				if (loseRevokeResponse)
					throw new ApiResponseError({
						status: 503,
						message: "Revocation response was lost.",
					});
				return;
			}
			if (path.includes("/resource-grants/")) {
				grants = grants.map((row) => ({ ...row, status: "revoked" }));
				return;
			}
			if (rejectRevoke)
				throw new ApiResponseError({
					status: 503,
					message: "Please retry later.",
				});
			statuses.set(account, "revoked");
		},
	},
};
mock.module("../../../state/backend-state", () => ({
	useBackend: () => backend,
	useBackendReady: () => true,
}));
mock.module("../../../hooks/use-hub", () => ({
	useHub: () => ({ hub: { standalone: { enabled } }, refetch: async () => {} }),
}));
mock.module("../../../hooks/use-invoke", () => ({
	useInvoke: () => ({
		data: { hub: "api.example.com", id: "profile-1" },
		isError: false,
	}),
}));
mock.module("react-oidc-context", () => ({
	useAuth: () => ({
		isAuthenticated: authenticated,
		isLoading: false,
		user: { profile: { sub: account, iss: "issuer-1" } },
	}),
}));
mock.module("@flow-like/locales", () => ({
	useTranslation: () => ({
		t: (_key: string, fallback: string, args?: { name?: string }) =>
			fallback.replace("{{name}}", args?.name ?? ""),
		i18n: { language: "en" },
	}),
}));
mock.module("./device-management-dialog", () => ({
	DeviceManagementDialog: (props: ManagementDialogProps) => {
		managementDialog = props;
		return (
			<div data-testid="management-inspection">
				<button type="button" onClick={props.onClose}>
					Close management
				</button>
			</div>
		);
	},
}));

const { createRoot } = await import("react-dom/client");
const { DevicesPage } = await import("./devices-page");
const container = window.document.createElement("div");
window.document.body.appendChild(container);
const root = createRoot(container as unknown as HTMLElement);
const client = new QueryClient({
	defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
});

async function render(projectId?: string) {
	await act(async () => {
		root.render(
			<QueryClientProvider client={client}>
				<DevicesPage projectId={projectId} />
			</QueryClientProvider>,
		);
	});
	await settle();
}
async function settle() {
	await act(async () => {
		await new Promise((resolve) => setTimeout(resolve, 15));
	});
}
async function clickButton(text: string, scope = window.document.body) {
	const button = [...scope.querySelectorAll("button")].find(
		(item) => item.textContent === text,
	);
	expect(button).toBeDefined();
	await act(async () => {
		button?.click();
	});
	await settle();
}
async function setField(label: string, value: string) {
	const container = [...window.document.querySelectorAll("label")].find(
		(item) => item.textContent?.includes(label),
	);
	const input = container?.querySelector("input,textarea");
	expect(input).toBeDefined();
	await act(async () => {
		const prototype =
			input?.tagName === "TEXTAREA"
				? window.HTMLTextAreaElement.prototype
				: window.HTMLInputElement.prototype;
		Object.getOwnPropertyDescriptor(prototype, "value")?.set?.call(
			input,
			value,
		);
		input?.dispatchEvent(new window.Event("input", { bubbles: true }));
	});
}
async function importPlacement(text: string | Promise<string>) {
	const input = window.document.querySelector('input[type="file"]');
	expect(input).not.toBeNull();
	await act(async () => {
		Object.defineProperty(input, "files", {
			configurable: true,
			value: [{ size: 100, text: async () => text }],
		});
		input?.dispatchEvent(new window.Event("change", { bubbles: true }));
	});
	await settle();
}
afterAll(async () => {
	await act(async () => root.unmount());
	client.clear();
	mock.restore();
	await window.happyDOM.close();
});

test("gates inventory, confirms revocation, keeps errors actionable, and isolates account switches", async () => {
	authenticated = false;
	await render();
	expect(container.textContent).toContain("Sign in");
	expect(calls).toEqual([]);
	authenticated = true;
	enabled = false;
	await render();
	expect(container.textContent).toContain("not enabled");
	expect(calls).toEqual([]);
	enabled = true;
	await render();
	expect(container.textContent).toContain("owner-a server");
	expect(container.textContent).toContain("No heartbeat received");
	expect(
		client
			.getQueryCache()
			.getAll()
			.find((query) => query.queryKey[0] === "devices")?.meta?.persist,
	).toBe(false);
	await clickButton("Revoke access");
	expect(calls.filter(([method]) => method === "DELETE")).toHaveLength(0);
	expect(window.document.activeElement?.textContent).toBe("Cancel");
	const dialog = window.document.querySelector('[role="alertdialog"]');
	expect(dialog?.textContent).toContain("Existing local services keep running");
	await clickButton("Revoke access", dialog as typeof window.document.body);
	expect(
		window.document.querySelector('[role="alertdialog"]')?.textContent,
	).toContain("Please retry later.");
	expect(calls.filter(([method]) => method === "DELETE")).toEqual([
		["DELETE", "devices/owner-a-device"],
	]);
	// An open confirmation must not carry across an account change.
	account = "owner-b";
	await render();
	expect(window.document.querySelector('[role="alertdialog"]')).toBeNull();
	expect(container.textContent).toContain("owner-b server");
	expect(container.textContent).not.toContain("owner-a server");
	rejectRevoke = false;
	await clickButton("Revoke access");
	await clickButton(
		"Revoke access",
		window.document.querySelector(
			'[role="alertdialog"]',
		) as typeof window.document.body,
	);
	expect(window.document.querySelector('[role="alertdialog"]')).toBeNull();
	expect(container.textContent).toContain("Access revoked");
	expect(calls.at(-2)).toEqual(["DELETE", "devices/owner-b-device"]);
	expect(
		[...container.querySelectorAll("button")].some(
			(button) => button.textContent === "Revoke access",
		),
	).toBe(false);
});

test("locked device cards show expiration warnings and notification links open the matching device", async () => {
	account = "certificate-owner";
	window.history.replaceState(
		null,
		"",
		"/settings/devices?device=certificate-owner-device",
	);
	await render();
	expect(container.textContent).toContain(
		"service certificate expiring within 7 days",
	);
	expect(container.textContent).toContain("Earliest expiry");
	expect(activeManagementDialog().device.device_id).toBe(
		"certificate-owner-device",
	);
	expect(
		client
			.getQueryCache()
			.getAll()
			.find(
				(query) =>
					query.queryKey[0] === "device-certificate-inventory" &&
					query.queryKey.includes(account),
			)?.meta?.persist,
	).toBe(false);
	await clickButton("Close management");
	window.history.replaceState(null, "", "/settings/devices");
	account = "after-certificate-owner";
	await render();
	expect(container.textContent).not.toContain(
		"service certificate expiring within 7 days",
	);
});

test("separates consent, recovers ambiguous outcomes, exports public bindings, and blocks stale authority", async () => {
	account = "resource-owner";
	statuses.clear();
	loseGrantResponse = true;
	loseBillingResponse = true;
	replacementBilling = true;
	await render();
	await clickButton("Hosted resources");
	await clickButton("Authorize a placement");
	const placement = JSON.stringify({
		id: "placement",
		deployment_id: "deployment",
		project_id: "offline-project",
		source: "offline",
		variables: { secret: "NEVER-EXPOSE-THIS" },
		project_path: "/private/path",
	});
	await importPlacement(placement);
	expect(window.document.body.textContent).not.toContain("NEVER-EXPOSE-THIS");
	await setField("Exact model Bit IDs", "model-bit");
	await clickButton("Approve model access");
	expect(posts).toHaveLength(1);
	expect(posts[0].body).toEqual({
		placement_id: "placement",
		deployment_id: "deployment",
		project_id: "offline-project",
		app_id: null,
		model_ids: ["model-bit"],
		max_instances: 1,
		expires_at: expect.any(Number),
	});
	expect(window.document.body.textContent).toContain(
		"The request may already have taken effect",
	);
	expect(window.document.body.textContent).toContain(
		"Hosted requests require a separate personal billing approval",
	);
	expect(window.document.body.textContent).not.toContain("Copy public binding");
	await clickButton("Review personal billing");
	await setField("Personal allowance (EUR)", "0.0000001");
	await clickButton("Approve personal billing");
	expect(posts).toHaveLength(1);
	await setField("Personal allowance (EUR)", "10.000001");
	await clickButton("Approve personal billing");
	expect(posts).toHaveLength(2);
	expect(posts[1].body.limit_micros).toBe(10_000_001);
	expect(window.document.body.textContent).toContain("€5.00");
	expect(window.document.body.textContent).toContain("€0.000001");
	expect(window.document.body.textContent).toContain("€1.00");
	expect(window.document.body.textContent).toContain(
		"Configuration exports use the recorded approvals",
	);
	expect(window.document.body.textContent).toContain("Lease may have expired");
	const binding = JSON.parse(
		window.document.querySelector("pre")?.textContent ?? "null",
	);
	expect(binding).toEqual({
		resource_grant: {
			grant_id: "grant-1",
			authz_version: 1,
			billing_grant_id: "previous-billing",
			billing_authz_version: 1,
		},
	});
	expect(
		JSON.stringify(
			client.getQueryData([
				"device-resources",
				"https://api.example.com",
				"issuer-1",
				account,
				"profile-1",
				`${account}-device`,
			]),
		),
	).not.toContain("NEVER-EXPOSE-THIS");
	const cached = client
		.getQueryCache()
		.getAll()
		.find((query) => query.queryKey[0] === "device-resources");
	expect(cached?.meta?.persist).toBe(false);
	expect(JSON.stringify(cached?.state.data)).not.toContain(
		"PUBLIC-SIGNED-RECEIPT",
	);
	let copied = "";
	Object.defineProperty(window.navigator, "clipboard", {
		configurable: true,
		value: {
			writeText: async (value: string) => {
				copied = value;
			},
		},
	});
	await clickButton("Copy public binding");
	expect(JSON.parse(copied)).toEqual(binding);
	resourceError = true;
	await clickButton(
		"Refresh",
		window.document.querySelector(
			'[role="dialog"]',
		) as typeof window.document.body,
	);
	expect(window.document.body.textContent).toContain(
		"Resource service unavailable",
	);
	expect(window.document.body.textContent).not.toContain("Copy public binding");
	expect(
		[...window.document.querySelectorAll("button")].find(
			(button) => button.textContent === "Authorize a placement",
		)?.disabled,
	).toBe(true);
	resourceError = false;
	await clickButton(
		"Refresh",
		window.document.querySelector(
			'[role="dialog"]',
		) as typeof window.document.body,
	);
	statuses.set(account, "revoked");
	await act(async () => {
		await client.invalidateQueries({ queryKey: ["devices"] });
	});
	await settle();
	expect(window.document.body.textContent).toContain("This device is revoked");
	expect(window.document.body.textContent).not.toContain("Copy public binding");
	loseRevokeResponse = true;
	await clickButton("Revoke billing approval");
	expect(
		window.document.querySelector('[role="alertdialog"]')?.textContent,
	).toContain("requests already dispatched can still settle");
	await clickButton("Revoke approval");
	expect(window.document.querySelector('[role="alertdialog"]')).toBeNull();
	expect(window.document.body.textContent).toContain(
		"confirms that this approval is revoked",
	);
	account = "another-owner";
	await render();
	expect(window.document.querySelector('[role="dialog"]')).toBeNull();
	expect(window.document.body.textContent).not.toContain("offline-project");
});

test("late file imports and mutation completions cannot cross account boundaries", async () => {
	account = "late-owner";
	grants = [];
	billing = [];
	loseGrantResponse = false;
	loseBillingResponse = false;
	replacementBilling = false;
	resourceError = false;
	await render();
	await clickButton("Hosted resources");
	await clickButton("Authorize a placement");
	let finishImport!: (value: string) => void;
	const oldFile = new Promise<string>((resolve) => {
		finishImport = resolve;
	});
	await importPlacement(oldFile);
	const placement = (id: string) =>
		JSON.stringify({
			id,
			deployment_id: "deployment",
			project_id: "project",
			source: "offline",
		});
	await importPlacement(placement("new-placement"));
	await act(async () => finishImport(placement("old-placement")));
	await settle();
	expect(window.document.body.textContent).toContain("new-placement");
	expect(window.document.body.textContent).not.toContain("old-placement");
	await setField("Exact model Bit IDs", "model-bit");
	latePost = () => {};
	await clickButton("Approve model access");
	const count = posts.length;
	await clickButton("Approve model access");
	expect(posts).toHaveLength(count);
	account = "after-late-owner";
	await render();
	expect(window.document.querySelector('[role="dialog"]')).toBeNull();
	await act(async () => {
		latePost?.();
		latePost = undefined;
	});
	await settle();
	expect(
		client
			.getQueryCache()
			.getAll()
			.filter(
				(query) =>
					query.queryKey[0] === "device-resources" &&
					query.queryKey.includes("late-owner"),
			),
	).toHaveLength(0);
});

function inspectedPlacement(id: string, projectId: string): PlacementStatus {
	return {
		id,
		project_id: projectId,
		deployment_id: `deployment-${id}`,
		revision: `revision-${id}`,
		desired_state: "running",
		observed_state: "running",
		config_revision: 4,
		intent_revision: 5,
		applied_revision: 4,
		desired_replicas: 3,
		running_replicas: 2,
		ready_replicas: 2,
		max_replicas: 4,
		replicas: [],
	};
}
async function reportInspection(
	dialog: ManagementDialogProps,
	placements: PlacementStatus[],
	deviceId = dialog.device.device_id,
) {
	await act(async () => {
		dialog.onInspection?.({
			device_id: deviceId,
			boot_id: "boot-1",
			placements,
		});
	});
	await settle();
}
function activeManagementDialog(): ManagementDialogProps {
	if (!managementDialog) throw new Error("No management dialog was rendered.");
	return managementDialog;
}

test("project devices show only inspected project placements and refresh the observed result", async () => {
	account = "project-owner";
	await render("project-a");
	expect(container.textContent).toContain("Project devices");
	expect(container.textContent).toContain(
		"Unlock to check whether this project is deployed here.",
	);
	await clickButton("Manage");
	const dialog = activeManagementDialog();
	expect(dialog.projectId).toBe("project-a");
	await reportInspection(
		dialog,
		[inspectedPlacement("wrong-device", "project-a")],
		"another-device",
	);
	expect(container.textContent).not.toContain("wrong-device");
	await reportInspection(dialog, [
		inspectedPlacement("visible-placement", "project-a"),
		inspectedPlacement("private-other-placement", "project-b"),
	]);
	expect(container.textContent).toContain("visible-placement: running");
	expect(container.textContent).toContain(
		"Revision revision-visible-placement",
	);
	expect(container.textContent).toContain("configuration 4, applied 4");
	expect(container.textContent).toContain("2 ready / 3 requested replicas");
	expect(container.textContent).toContain("Reconnect to refresh.");
	expect(container.textContent).not.toContain("private-other-placement");
	expect(container.textContent).not.toContain("Unlock to check");
	await clickButton("Close management");
	expect(container.textContent).toContain("visible-placement: running");
	await clickButton("Manage");
	await reportInspection(activeManagementDialog(), [
		inspectedPlacement("still-other-placement", "project-b"),
	]);
	expect(container.textContent).toContain(
		"No placements for this project were visible at that time.",
	);
	expect(container.textContent).not.toContain("visible-placement");
	expect(container.textContent).not.toContain("still-other-placement");
	await clickButton("Close management");
});

test("project and account changes discard device observations and ignore late inspection callbacks", async () => {
	account = "project-switch-owner";
	await render("project-a");
	await clickButton("Manage");
	const firstProjectDialog = activeManagementDialog();
	await reportInspection(firstProjectDialog, [
		inspectedPlacement("first-project-observation", "project-a"),
	]);
	expect(container.textContent).toContain("first-project-observation");
	await render("project-b");
	expect(
		container.querySelector('[data-testid="management-inspection"]'),
	).toBeNull();
	expect(container.textContent).toContain("Unlock to check");
	expect(container.textContent).not.toContain("first-project-observation");
	await reportInspection(firstProjectDialog, [
		inspectedPlacement("late-project-observation", "project-a"),
	]);
	expect(container.textContent).not.toContain("late-project-observation");
	await render("project-a");
	expect(container.textContent).toContain("Unlock to check");
	expect(container.textContent).not.toContain("first-project-observation");
	await clickButton("Manage");
	const oldAccountDialog = activeManagementDialog();
	await reportInspection(oldAccountDialog, [
		inspectedPlacement("old-account-observation", "project-a"),
	]);
	account = "next-project-owner";
	await render("project-a");
	expect(
		container.querySelector('[data-testid="management-inspection"]'),
	).toBeNull();
	expect(container.textContent).toContain("next-project-owner server");
	expect(container.textContent).toContain("Unlock to check");
	expect(container.textContent).not.toContain("old-account-observation");
	await reportInspection(oldAccountDialog, [
		inspectedPlacement("late-account-observation", "project-a"),
	]);
	expect(container.textContent).not.toContain("late-account-observation");
	account = "project-switch-owner";
	await render("project-a");
	expect(container.textContent).toContain("Unlock to check");
	expect(container.textContent).not.toContain("old-account-observation");
	expect(
		JSON.stringify(
			client
				.getQueryCache()
				.getAll()
				.map((query) => query.state.data),
		),
	).not.toContain("observation");
});

test("resource inventory distinguishes metadata validation from service workload leases", async () => {
	account = "validation-owner";
	grants = [];
	billing = [];
	resourceError = false;
	const lease = {
		device_id: `${account}-device`,
		grant_id: "online-grant",
		registered_at: Math.floor(Date.now() / 1000),
		lease_expires_at: Math.floor(Date.now() / 1000) + 600,
	};
	resourceInstances = [
		{ ...lease, instance_id: "legacy-service" },
		{
			...lease,
			instance_id: "validation-attempt",
			purpose: "rollout_validation",
		},
	];
	await render();
	await clickButton("Hosted resources");
	const dialog = window.document.querySelector('[role="dialog"]');
	expect(dialog?.textContent).toContain(
		"Service workloads: 1 · Metadata validations: 1",
	);
	expect(dialog?.textContent).toContain(
		"do not count toward the placement's service instance limit",
	);
	const leases = [...(dialog?.querySelectorAll("li") ?? [])];
	const service = leases.find((row) =>
		row.textContent?.includes("legacy-service"),
	);
	const validation = leases.find((row) =>
		row.textContent?.includes("validation-attempt"),
	);
	expect(service?.textContent).toContain("Service workload");
	expect(service?.textContent).not.toContain("Metadata validation");
	expect(validation?.textContent).toContain("Metadata validation");
	expect(validation?.textContent).not.toContain("Service workload");
	account = "after-validation-owner";
	resourceInstances = undefined;
	await render();
});
