import { afterAll, afterEach, beforeEach, expect, mock, test } from "bun:test";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { Window } from "happy-dom";
import { type ComponentProps, act } from "react";
import { type Root, createRoot } from "react-dom/client";
import { ApiResponseError } from "../../../lib/api-error";
import type {
	TeamsBotAccess,
	TeamsBotConnection,
	TeamsBotSetup,
} from "../../../lib/teams-bot";

const tenant = "11111111-1111-1111-1111-111111111111";
const clientId = "22222222-2222-2222-2222-222222222222";
const approver = "33333333-3333-3333-3333-333333333333";
const consentUrl = `https://login.microsoftonline.com/${tenant}/adminconsent?client_id=${clientId}`;
let response: TeamsBotConnection;
let offline: boolean;
let failure: { error: Error; status?: string } | undefined;
let access: TeamsBotAccess;
let accessFailure: Error | undefined;
const fields = new Map<
	string,
	ComponentProps<"input"> | ComponentProps<"textarea">
>();
const getTeamsBot = mock(async function getTeamsBot() {
	return response;
});
const setupTeamsBot = mock(
	async (_app: string, _event: string, input: TeamsBotSetup) => {
		if (failure) {
			if (failure.status) response = { ...response, status: failure.status };
			throw failure.error;
		}
		const { client_secret: _secret, ...publicSettings } = input;
		response = {
			...response,
			...publicSettings,
			configured: true,
			status: "ready",
		};
		return response;
	},
);
const disconnectTeamsBot = mock(async () => {
	response = { ...response, configured: false, status: "disconnected" };
	return response;
});
const rotateTeamsBotSecret = mock(async () => {
	response = { ...response, secret_expires_at: "2027-01-01T00:00:00Z" };
	return response;
});
const getTeamsBotPackage = mock(async () => ({
	filename: "support-bot.zip",
	base64: "UEs=",
}));
const getTeamsBotAccess = mock(async function getTeamsBotAccess() {
	if (accessFailure) throw accessFailure;
	return access;
});
const onConfigUpdate = mock(() => {});
const eventState = {
	getTeamsBot,
	setupTeamsBot,
	disconnectTeamsBot,
	rotateTeamsBotSecret,
	getTeamsBotPackage,
	getTeamsBotAccess,
};
const backend: {
	isOffline: () => Promise<boolean>;
	eventState: Partial<typeof eventState>;
} = {
	isOffline: async function isOffline() {
		return offline;
	},
	eventState,
};

const documentDescriptor = Object.getOwnPropertyDescriptor(
	globalThis,
	"document",
);
Object.assign(globalThis, { document: new Window().document });
const actual = {
	backend: { ...(await import("../../../state/backend-state")) },
	input: { ...(await import("../../ui/input")) },
	textarea: { ...(await import("../../ui/textarea")) },
};
mock.module("../../../state/backend-state", () => ({
	...actual.backend,
	useBackend: () => backend,
}));
mock.module("../../ui/input", () => ({
	...actual.input,
	Input: (props: ComponentProps<"input">) => {
		if (props.id) fields.set(props.id, props);
		return <input {...props} />;
	},
}));
mock.module("../../ui/textarea", () => ({
	...actual.textarea,
	Textarea: (props: ComponentProps<"textarea">) => {
		if (props.id) fields.set(props.id, props);
		return <textarea {...props} />;
	},
}));
// Loaded once here: the first import pulls the UI library barrel, which is slow
// on a busy machine and would otherwise count against the first test's timeout.
const teamsModule = "./teams.tsx?teams-behavior-test";
const { TeamsConfig } = await import(teamsModule);
if (documentDescriptor)
	Object.defineProperty(globalThis, "document", documentDescriptor);
else Reflect.deleteProperty(globalThis, "document");

// Radix (radio group, collapsible, alert dialog) reads these from the global scope.
const WINDOW_GLOBALS = [
	"HTMLElement",
	"HTMLInputElement",
	"HTMLButtonElement",
	"Element",
	"Node",
	"Text",
	"DocumentFragment",
	"NodeFilter",
	"MutationObserver",
	"ResizeObserver",
	"Event",
	"CustomEvent",
	"KeyboardEvent",
	"FocusEvent",
	"MouseEvent",
	"PointerEvent",
] as const;
const savedGlobals = [
	"window",
	"document",
	"navigator",
	"getComputedStyle",
	"requestAnimationFrame",
	"cancelAnimationFrame",
	"IS_REACT_ACT_ENVIRONMENT",
	...WINDOW_GLOBALS,
].map(
	(key) => [key, Object.getOwnPropertyDescriptor(globalThis, key)] as const,
);
let browser: Window;
let root: Root;
let queryClient: QueryClient;
beforeEach(() => {
	browser = new Window({ url: "https://flow-like.test/settings" });
	Object.assign(browser, { SyntaxError, TypeError });
	const globals: Record<string, unknown> = {
		window: browser,
		document: browser.document,
		navigator: browser.navigator,
		getComputedStyle: browser.getComputedStyle.bind(browser),
		requestAnimationFrame: (callback: FrameRequestCallback) =>
			setTimeout(() => callback(0), 0),
		cancelAnimationFrame: (id: number) => clearTimeout(id),
		IS_REACT_ACT_ENVIRONMENT: true,
	};
	for (const key of WINDOW_GLOBALS) globals[key] = browser[key];
	for (const [key, value] of Object.entries(globals))
		Object.defineProperty(globalThis, key, {
			configurable: true,
			writable: true,
			value,
		});
	const container = browser.document.createElement("div");
	browser.document.body.appendChild(container);
	root = createRoot(container as never);
	queryClient = new QueryClient({
		defaultOptions: { queries: { retry: false } },
	});
	// No `permissions`: older servers omit the field.
	response = {
		managed_available: false,
		configured: false,
		status: "not_configured",
		endpoint: "https://api.example.com/api/v1/sink/trigger/teams/connection",
		connection_id: "connection",
		mode: null,
		name: "Support",
		description: "",
		customer_tenant_id: "",
		home_tenant_id: "",
		client_id: "",
		secret_expires_at: null,
		allowed_responders: [],
	};
	offline = false;
	failure = undefined;
	access = { status: "ready" };
	accessFailure = undefined;
	fields.clear();
	onConfigUpdate.mockClear();
	backend.eventState = eventState;
	for (const method of Object.values(eventState)) method.mockClear();
});
afterEach(async () => {
	await act(async () => root.unmount());
	queryClient.clear();
	await browser.happyDOM.close();
	for (const [key, descriptor] of savedGlobals) {
		if (descriptor) Object.defineProperty(globalThis, key, descriptor);
		else Reflect.deleteProperty(globalThis, key);
	}
});
afterAll(() => {
	mock.restore();
	mock.module("../../../state/backend-state", () => actual.backend);
	mock.module("../../ui/input", () => actual.input);
	mock.module("../../ui/textarea", () => actual.textarea);
});

async function settle() {
	await act(async () => {
		await new Promise((resolve) => setTimeout(resolve, 20));
	});
}
async function render(eventId = "event", isEditing = true) {
	await act(async () =>
		root.render(
			<QueryClientProvider client={queryClient}>
				<TeamsConfig
					appId="app"
					boardId="board"
					nodeId="node"
					node={{} as never}
					eventId={eventId}
					config={{ sink_type: "teams" }}
					isEditing={isEditing}
					onConfigUpdate={onConfigUpdate}
				/>
			</QueryClientProvider>,
		),
	);
	await settle();
	await settle();
}
function button(text: string) {
	const found = Array.from(browser.document.querySelectorAll("button")).find(
		(node) => node.textContent?.trim() === text,
	);
	if (!found) throw new Error(`Button not found: ${text}`);
	return found;
}
async function click(text: string) {
	await act(async () => button(text).click());
	await settle();
}
function radio(mode: string) {
	const found = browser.document.querySelector(
		`button[role="radio"][value="${mode}"]`,
	);
	if (!found) throw new Error(`Radio not found: ${mode}`);
	return found as unknown as HTMLButtonElement;
}
function permissionSwitch(permission: string) {
	const found = browser.document.querySelector(
		`button[role="switch"][id$="-${permission}"]`,
	);
	if (!found) throw new Error(`Switch not found: ${permission}`);
	return found as unknown as HTMLButtonElement;
}
async function toggle(permission: string) {
	await act(async () => permissionSwitch(permission).click());
	await settle();
}
function pageText() {
	return browser.document.body.textContent ?? "";
}
function accessRow() {
	return browser.document.querySelector("output")?.textContent;
}
function alerts() {
	return Array.from(browser.document.querySelectorAll('[role="alert"]')).map(
		(node) => node.textContent ?? "",
	);
}
function field(suffix: string) {
	const found = [...fields]
		.reverse()
		.find(([id]) => id.endsWith(`-${suffix}`))?.[1];
	if (!found) throw new Error(`Field not found: ${suffix}`);
	return found as Record<string, unknown> & ComponentProps<"input">;
}
async function enter(suffix: string, value: string) {
	const change = field(suffix).onChange;
	if (!change) throw new Error(`Field cannot be edited: ${suffix}`);
	await act(async () => change({ target: { value } } as never));
}
async function customerCredentials() {
	await enter("tenant", tenant);
	await enter("home", tenant);
	await enter("client", clientId);
	await enter("secret", "private-value");
}
function configured(mode: TeamsBotConnection["mode"] = "customer_teams") {
	response = {
		...response,
		configured: true,
		status: "ready",
		mode,
		customer_tenant_id: tenant,
		home_tenant_id: mode === "flow_like_managed" ? "" : tenant,
		client_id: mode === "flow_like_managed" ? "" : clientId,
	};
}

test("new and offline events explain the prerequisite without fetching bot setup", async () => {
	await render("");
	expect(browser.document.body.textContent).toContain("Save this Teams event");
	expect(getTeamsBot).not.toHaveBeenCalled();
	offline = true;
	queryClient.clear();
	await render();
	expect(browser.document.body.textContent).toContain("Sync this app");
	expect(getTeamsBot).not.toHaveBeenCalled();
}, 15_000);

test("an unavailable managed path defaults to customer setup and details the selected path", async () => {
	await render();
	expect(radio("flow_like_managed").disabled).toBe(true);
	expect(radio("customer_teams").getAttribute("aria-checked")).toBe("true");
	const headings = Array.from(browser.document.querySelectorAll("dt")).map(
		(node) => node.textContent,
	);
	for (const heading of [
		"Pros",
		"Cons",
		"You need",
		"What to expect",
		"Microsoft sign-in",
	])
		expect(headings.filter((text) => text === heading)).toHaveLength(1);
	const text = browser.document.body.textContent ?? "";
	expect(text).toContain("How to find your tenant ID");
	expect(field("endpoint").value).toBe(response.endpoint);
	expect(field("endpoint").readOnly).toBe(true);
	const secret = field("secret");
	expect(secret.type).toBe("password");
	expect(secret.autoComplete).toBe("off");
	expect(secret["data-1p-ignore"]).toBe(true);
	expect(secret["data-lpignore"]).toBe("true");
	expect(text).not.toContain("Download Teams app (.zip)");
});

test("customer setup marks invalid fields, then saves credentials and approvers separately from event config", async () => {
	await render();
	await click("Connect bot");
	expect(setupTeamsBot).not.toHaveBeenCalled();
	expect(alerts().join(" ")).toContain("tenant ID");
	expect(field("tenant")["aria-invalid"]).toBe(true);
	expect(field("client")["aria-invalid"]).toBe(true);
	expect(field("name")["aria-invalid"]).toBe(false);
	await customerCredentials();
	expect(field("tenant")["aria-invalid"]).toBe(false);
	await click("Approvals and actions");
	await enter("approvers", `${approver},\n${tenant}`);
	await click("Connect bot");
	expect(setupTeamsBot).toHaveBeenCalledWith("app", "event", {
		mode: "customer_teams",
		name: "Support",
		description: "",
		customer_tenant_id: tenant,
		home_tenant_id: tenant,
		client_id: clientId,
		client_secret: "private-value",
		allowed_responders: [approver, tenant],
		permissions: [],
	});
	expect(field("secret").value).toBe("");
	expect(onConfigUpdate).not.toHaveBeenCalled();
	expect(browser.document.body.textContent).toContain(
		"Download Teams app (.zip)",
	);
	expect(browser.document.body.textContent).toContain("Connected");
	expect(browser.document.querySelector("fieldset")?.disabled).toBe(true);
});

test("switching to the managed path drops customer credentials", async () => {
	response.managed_available = true;
	response.mode = "customer_teams";
	await render();
	await customerCredentials();
	await act(async () => radio("flow_like_managed").click());
	expect(browser.document.querySelector('input[type="password"]')).toBeNull();
	await act(async () => radio("customer_azure").click());
	expect(field("client").value).toBe("");
	expect(field("home").value).toBe("");
	expect(field("secret").value).toBe("");
	expect(field("tenant").value).toBe(tenant);
});

test("managed setup needs the Teams tenant and does not request customer credentials", async () => {
	response.managed_available = true;
	await render();
	expect(browser.document.querySelector('input[type="password"]')).toBeNull();
	await enter("tenant", tenant);
	await click("Create bot");
	expect(setupTeamsBot).toHaveBeenCalledWith("app", "event", {
		mode: "flow_like_managed",
		name: "Support",
		description: "",
		customer_tenant_id: tenant,
		home_tenant_id: "",
		client_id: "",
		client_secret: "",
		allowed_responders: [],
		permissions: [],
	});
	expect(browser.document.body.textContent).toContain(
		"Download Teams app (.zip)",
	);
});

test("only the selected management path shows its details", async () => {
	await render();
	const facts = () =>
		Array.from(browser.document.querySelectorAll("dl")).map(
			(node) => node.textContent ?? "",
		);
	expect(facts()).toHaveLength(1);
	expect(facts()[0]).toContain("You own the bot identity");
	expect(pageText()).toContain("Not enabled on this server");
	expect(pageText()).toContain(
		"Connect an Azure Bot resource your organization owns.",
	);
	expect(pageText()).not.toContain("Full control of the Azure resource");
	expect(
		radio("customer_azure").getAttribute("aria-describedby"),
	).not.toContain("-facts");
	await act(async () => radio("customer_azure").click());
	expect(facts()).toHaveLength(1);
	expect(facts()[0]).toContain("Full control of the Azure resource");
	expect(pageText()).not.toContain("You own the bot identity");
	expect(radio("customer_azure").getAttribute("aria-describedby")).toContain(
		"-facts",
	);
});

test("read permissions are saved with the bot, and reading messages explains what changes", async () => {
	await render();
	await customerCredentials();
	const note = "Teams sends the bot every message";
	expect(permissionSwitch("read_messages").getAttribute("aria-checked")).toBe(
		"false",
	);
	expect(pageText()).toContain("ChannelMessage.Read.Group");
	expect(pageText()).not.toContain(note);
	await toggle("conversation_details");
	expect(pageText()).not.toContain(note);
	await toggle("read_messages");
	expect(pageText()).toContain(note);
	await toggle("read_messages");
	expect(pageText()).not.toContain(note);
	await toggle("read_messages");
	expect(accessRow()).toBeUndefined();
	await click("Connect bot");
	expect(setupTeamsBot.mock.calls.at(-1)?.[2]).toEqual({
		mode: "customer_teams",
		name: "Support",
		description: "",
		customer_tenant_id: tenant,
		home_tenant_id: tenant,
		client_id: clientId,
		client_secret: "private-value",
		allowed_responders: [],
		permissions: ["read_messages", "conversation_details"],
	});
	expect(permissionSwitch("read_messages").getAttribute("aria-checked")).toBe(
		"true",
	);
	expect(permissionSwitch("meeting_details").getAttribute("aria-checked")).toBe(
		"false",
	);
	expect(getTeamsBotAccess).toHaveBeenCalledWith("app", "event");
	expect(accessRow()).toContain("Ready");
});

test("a permissions-only change on a connected bot saves without confirmation", async () => {
	configured();
	await render();
	await toggle("meeting_details");
	await click("Save bot settings");
	expect(pageText()).not.toContain("Change the connected bot?");
	expect(setupTeamsBot.mock.calls.at(-1)?.[2].permissions).toEqual([
		"meeting_details",
	]);
});

test("the Graph access row reports admin consent, readiness and failures, and checks again", async () => {
	configured("flow_like_managed");
	response.managed_available = true;
	response.permissions = ["read_messages"];
	access = {
		status: "consent_required",
		consent_url: consentUrl,
		message: "The tenant has not approved the bot.",
	};
	await render();
	expect(accessRow()).toContain("Needs admin consent");
	expect(accessRow()).toContain("The tenant has not approved the bot.");
	const consent = browser.document.querySelector(`a[href="${consentUrl}"]`);
	expect(consent?.textContent).toContain("Grant admin consent");
	expect(consent?.getAttribute("target")).toBe("_blank");

	access = { status: "ready" };
	await click("Check again");
	expect(accessRow()).toContain("Ready");
	expect(browser.document.querySelector(`a[href="${consentUrl}"]`)).toBeNull();

	access = { status: "error", message: "Microsoft Graph returned 503." };
	await click("Check again");
	expect(accessRow()).toContain("Check failed");
	expect(accessRow()).toContain("Microsoft Graph returned 503.");

	accessFailure = new Error("The access check could not reach the server.");
	await click("Check again");
	expect(accessRow()).toContain("Check failed");
	expect(accessRow()).toContain("could not reach the server");
	expect(getTeamsBotAccess).toHaveBeenCalledTimes(4);
});

test("the Graph access row needs saved read permissions", async () => {
	configured();
	await render();
	expect(accessRow()).toBeUndefined();
	expect(getTeamsBotAccess).not.toHaveBeenCalled();
});

test("servers without an access check hide the row but keep the saved permissions", async () => {
	configured();
	response.permissions = ["meeting_details"];
	const { getTeamsBotAccess: _unsupported, ...older } = eventState;
	backend.eventState = older;
	await render();
	expect(permissionSwitch("meeting_details").getAttribute("aria-checked")).toBe(
		"true",
	);
	expect(accessRow()).toBeUndefined();
	expect(getTeamsBotAccess).not.toHaveBeenCalled();
});

test("metadata edits save directly; a changed identity keeps its secret requirement", async () => {
	configured();
	await render();
	expect(field("secret").placeholder).toContain("Leave blank");
	await enter("name", "Updated Support");
	await click("Save bot settings");
	expect(setupTeamsBot.mock.calls.at(-1)?.[2].client_secret).toBe("");
	expect(browser.document.body.textContent).not.toContain(
		"Change the connected bot?",
	);
	setupTeamsBot.mockClear();
	await enter("client", approver);
	await click("Save bot settings");
	expect(setupTeamsBot).not.toHaveBeenCalled();
	expect(alerts().join(" ")).toContain("secret value");
	expect(field("secret")["aria-invalid"]).toBe(true);
});

test("new credentials for a connected bot are confirmed before saving", async () => {
	configured();
	await render();
	await enter("secret", "rotated-value");
	await click("Save bot settings");
	expect(setupTeamsBot).not.toHaveBeenCalled();
	expect(browser.document.body.textContent).toContain(
		"Change the connected bot?",
	);
	await click("Cancel");
	expect(setupTeamsBot).not.toHaveBeenCalled();
	await click("Save bot settings");
	await click("Save changes");
	expect(setupTeamsBot.mock.calls.at(-1)?.[2].client_secret).toBe(
		"rotated-value",
	);
});

test("a failed setup keeps the draft, shows the error and refetches the status", async () => {
	await render();
	await customerCredentials();
	const fetches = getTeamsBot.mock.calls.length;
	const message = "Microsoft registration is still preparing. Retry setup.";
	failure = { error: new Error(message) };
	await click("Connect bot");
	expect(alerts()).toContain(message);
	expect(getTeamsBot.mock.calls.length).toBeGreaterThan(fetches);
	expect(browser.document.body.textContent).toContain("Not set up");
	expect(field("secret").value).toBe("private-value");
	expect(button("Connect bot").disabled).toBe(false);
});

test("API refusals are explained with a next step and the server's detail", async () => {
	response.managed_available = true;
	await render();
	await enter("tenant", tenant);
	const refusal = (status: number, message: string) =>
		new ApiResponseError({ status, code: `HTTP_${status}`, message });
	const cases = [
		[422, "This app already has 10 bots.", "limit of Flow-Like managed bots"],
		[429, "At most 5 setups per hour.", "Try again later"],
		[403, "Teams bots are not enabled.", "Ask your administrator"],
		[400, "Managed bots are not enabled.", "connect your own bot instead"],
	] as const;
	for (const [status, detail, guidance] of cases) {
		failure = { error: refusal(status, detail) };
		await click("Create bot");
		const text = alerts().join(" ");
		expect(text).toContain(guidance);
		expect(text).toContain(detail);
		expect(text).not.toContain(`HTTP_${status}`);
	}
	failure = {
		error: refusal(502, "Microsoft bot provisioning could not be reached."),
		status: "setup_failed",
	};
	await click("Create bot");
	expect(alerts().join(" ")).toContain("could not be reached");
	expect(browser.document.body.textContent).toContain("Setup failed");
});

test("saving is paused while the server is still provisioning", async () => {
	configured();
	response = { ...response, configured: false, status: "provisioning" };
	await render();
	expect(button("Connect bot").disabled).toBe(true);
	expect(browser.document.body.textContent).toContain(
		"Microsoft is preparing the bot",
	);
});

test("a result for a previous event is not applied after switching events", async () => {
	await render();
	await customerCredentials();
	let reject: (error: Error) => void = () => {};
	setupTeamsBot.mockImplementationOnce(
		() =>
			new Promise<TeamsBotConnection>((_resolve, fail) => {
				reject = fail;
			}),
	);
	await act(async () => button("Connect bot").click());
	await render("other");
	await act(async () => reject(new Error("Stale failure")));
	await settle();
	expect(alerts()).not.toContain("Stale failure");
	expect(button("Connect bot").disabled).toBe(false);
	expect(getTeamsBot).toHaveBeenCalledWith("app", "other");
});

test("the Teams app downloads under the backend filename", async () => {
	configured();
	const downloads: string[] = [];
	const createElement = browser.document.createElement.bind(browser.document);
	browser.document.createElement = ((tag: string) => {
		const element = createElement(tag);
		if (tag === "a")
			Object.defineProperty(element, "click", {
				value: () =>
					downloads.push((element as unknown as HTMLAnchorElement).download),
			});
		return element;
	}) as typeof browser.document.createElement;
	await render();
	const fetches = getTeamsBot.mock.calls.length;
	await click("Download Teams app (.zip)");
	expect(getTeamsBotPackage).toHaveBeenCalledWith("app", "event");
	expect(downloads).toEqual(["support-bot.zip"]);
	expect(getTeamsBot.mock.calls.length).toBe(fetches);
	expect(button("Download Teams app (.zip)").disabled).toBe(false);
});

test("renewing a managed bot secret refetches its expiry", async () => {
	configured("flow_like_managed");
	response.managed_available = true;
	await render();
	await click("Connection maintenance");
	await click("Renew bot secret");
	expect(rotateTeamsBotSecret).toHaveBeenCalledWith("app", "event");
	expect(browser.document.body.textContent).toContain(
		`Secret expires ${new Date("2027-01-01T00:00:00Z").toLocaleDateString()}`,
	);
});

test("read-only viewers cannot change, renew, or disconnect the bot", async () => {
	configured("flow_like_managed");
	response.managed_available = true;
	await render("event", false);
	expect(field("name").disabled).toBe(true);
	expect(field("tenant").disabled).toBe(true);
	expect(browser.document.querySelector("fieldset")?.disabled).toBe(true);
	await click("Approvals and actions");
	expect(field("approvers").disabled).toBe(true);
	expect(permissionSwitch("read_messages").disabled).toBe(true);
	await act(async () => permissionSwitch("read_messages").click());
	expect(permissionSwitch("read_messages").getAttribute("aria-checked")).toBe(
		"false",
	);
	await click("Connection maintenance");
	for (const label of [
		"Save bot settings",
		"Renew bot secret",
		"Disconnect bot…",
	]) {
		expect(button(label).disabled).toBe(true);
		await act(async () => button(label).click());
	}
	expect(setupTeamsBot).not.toHaveBeenCalled();
	expect(rotateTeamsBotSecret).not.toHaveBeenCalled();
	expect(disconnectTeamsBot).not.toHaveBeenCalled();
});

test("disconnect requires confirmation and then unlocks the management choice", async () => {
	configured();
	await render();
	await click("Connection maintenance");
	await click("Disconnect bot…");
	expect(disconnectTeamsBot).not.toHaveBeenCalled();
	expect(browser.document.body.textContent).toContain(
		"Your Microsoft registration remains",
	);
	await click("Cancel");
	expect(disconnectTeamsBot).not.toHaveBeenCalled();
	await click("Disconnect bot…");
	await click("Disconnect bot");
	expect(disconnectTeamsBot).toHaveBeenCalledWith("app", "event");
	expect(browser.document.querySelector("fieldset")?.disabled).toBe(false);
	expect(browser.document.body.textContent).not.toContain(
		"Download Teams app (.zip)",
	);
});
