import { afterAll, afterEach, beforeEach, expect, mock, test } from "bun:test";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { Window } from "happy-dom";
import { type ComponentProps, act } from "react";
import { type Root, createRoot } from "react-dom/client";
import { ApiResponseError } from "../../../lib/api-error";
import type { IInboundEmailAddress } from "../../../state/backend-state/event-state";

let response: IInboundEmailAddress;
let offline: boolean;
let writeError: string | undefined;
const inputs = new Map<string, ComponentProps<"input">>();
const getInboundEmailAddress = mock(
	async (_appId: string, _eventId: string) => response,
);
const updateInboundEmailAlias = mock(
	async (_appId: string, _eventId: string, alias: string | null) => {
		if (writeError) throw new Error(writeError);
		response = { ...response, alias };
		return response;
	},
);
const backend = {
	isOffline: async () => offline,
	eventState: { getInboundEmailAddress, updateInboundEmailAlias },
};

const documentDescriptor = Object.getOwnPropertyDescriptor(
	globalThis,
	"document",
);
Object.assign(globalThis, { document: new Window().document });
const actual = {
	backend: { ...(await import("../../../state/backend-state")) },
	input: { ...(await import("../../ui/input")) },
};
if (documentDescriptor)
	Object.defineProperty(globalThis, "document", documentDescriptor);
else Reflect.deleteProperty(globalThis, "document");
mock.module("../../../state/backend-state", () => ({
	...actual.backend,
	useBackend: () => backend,
}));
mock.module("../../ui/input", () => ({
	...actual.input,
	Input: (props: ComponentProps<"input">) => {
		if (props.id) inputs.set(props.id, props);
		return <input {...props} />;
	},
}));

const saved = [
	"window",
	"document",
	"navigator",
	"IS_REACT_ACT_ENVIRONMENT",
].map(
	(key) => [key, Object.getOwnPropertyDescriptor(globalThis, key)] as const,
);
let browser: Window;
let root: Root;
let client: QueryClient;
beforeEach(() => {
	browser = new Window({ url: "https://flow-like.test/settings" });
	Object.assign(browser, { SyntaxError, TypeError });
	for (const [key, value] of Object.entries({
		window: browser,
		document: browser.document,
		navigator: browser.navigator,
		IS_REACT_ACT_ENVIRONMENT: true,
	}))
		Object.defineProperty(globalThis, key, {
			configurable: true,
			writable: true,
			value,
		});
	root = createRoot(document.body);
	client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
	response = {
		configured: true,
		domain: "mail.example.com",
		address: "m-generated@mail.example.com",
		alias: null,
		active: true,
	};
	offline = false;
	writeError = undefined;
	inputs.clear();
	getInboundEmailAddress.mockClear();
	updateInboundEmailAlias.mockClear();
});
afterEach(async () => {
	await act(async () => root.unmount());
	client.clear();
	await browser.happyDOM.close();
	for (const [key, descriptor] of saved) {
		if (descriptor) Object.defineProperty(globalThis, key, descriptor);
		else Reflect.deleteProperty(globalThis, key);
	}
});
afterAll(() => {
	mock.restore();
	mock.module("../../../state/backend-state", () => actual.backend);
	mock.module("../../ui/input", () => actual.input);
});

async function settle() {
	await act(async () => {
		await new Promise((resolve) => setTimeout(resolve, 20));
	});
}
async function render(eventId: string | undefined = "event", isEditing = true) {
	const path = "./inbound-email.tsx?email-behavior-test";
	const { InboundEmailConfig } = await import(path);
	await act(async () =>
		root.render(
			<QueryClientProvider client={client}>
				<InboundEmailConfig
					appId="app"
					boardId="board"
					nodeId="node"
					node={{} as never}
					eventId={eventId}
					config={{ sink_type: "inbound_email" }}
					isEditing={isEditing}
					onConfigUpdate={() => {}}
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
async function enterAlias(value: string) {
	const onChange = inputs.get("inbound-email-alias")?.onChange;
	if (!onChange) throw new Error("Alias input is missing");
	await act(async () =>
		onChange({
			target: { value },
		} as never),
	);
}

test("a new or offline event does not fetch an address", async () => {
	await render("");
	expect(browser.document.body.textContent).toContain("Save the event");
	expect(getInboundEmailAddress).not.toHaveBeenCalled();
	offline = true;
	client.clear();
	await render();
	expect(browser.document.body.textContent).toContain("Sync this app");
	expect(getInboundEmailAddress).not.toHaveBeenCalled();
});

test("shows server configuration and pending address states without alias controls", async () => {
	response = { ...response, configured: false, address: null, domain: null };
	await render();
	expect(browser.document.body.textContent).toContain(
		"not enabled on this server",
	);
	expect(browser.document.querySelector("#inbound-email-alias")).toBeNull();
	response = { ...response, configured: true, domain: "mail.example.com" };
	await act(async () => button("Refresh address").click());
	await settle();
	expect(browser.document.body.textContent).toContain(
		"email address is pending",
	);
});

test("saves and removes an alias while keeping the generated address", async () => {
	await render();
	expect(inputs.get("inbound-email-address")?.readOnly).toBe(true);
	await enterAlias(" Invoices ");
	await act(async () => button("Save alias").click());
	await settle();
	expect(updateInboundEmailAlias).toHaveBeenLastCalledWith(
		"app",
		"event",
		"invoices",
	);
	expect(browser.document.body.textContent).toContain(
		"invoices@mail.example.com",
	);
	expect(inputs.get("inbound-email-address")?.value).toBe(
		"m-generated@mail.example.com",
	);
	await act(async () => button("Remove alias").click());
	await settle();
	expect(updateInboundEmailAlias).toHaveBeenLastCalledWith(
		"app",
		"event",
		null,
	);
	expect(inputs.get("inbound-email-alias")?.value).toBe("");
});

test("a failed address request can be retried", async () => {
	getInboundEmailAddress.mockImplementationOnce(async () => {
		throw new Error("Address service is unavailable");
	});
	await render();
	expect(
		browser.document.querySelector('[role="alert"]')?.textContent,
	).toContain("Address service is unavailable");
	await act(async () => button("Retry").click());
	await settle();
	expect(getInboundEmailAddress).toHaveBeenCalledTimes(2);
	expect(inputs.get("inbound-email-address")?.value).toBe(
		"m-generated@mail.example.com",
	);
});

test("invalid aliases cannot be submitted and conflicts keep the draft", async () => {
	await render();
	await enterAlias("wrong@domain.com");
	expect(button("Save alias").disabled).toBe(true);
	expect(inputs.get("inbound-email-alias")?.["aria-invalid"]).toBe(true);
	expect(browser.document.body.textContent).toContain(
		"Use 3 to 64 lowercase letters",
	);
	await enterAlias("postmaster");
	expect(browser.document.body.textContent).toContain("This alias is reserved");
	await enterAlias("invoices");
	writeError = "Alias is already in use";
	await act(async () => button("Save alias").click());
	await settle();
	expect(browser.document.body.textContent).toContain(
		"Alias is already in use",
	);
	expect(inputs.get("inbound-email-alias")?.value).toBe("invoices");
});

test("alias refusals show the server's message without its error code", async () => {
	await render();
	await enterAlias("invoices");
	updateInboundEmailAlias.mockImplementationOnce(async () => {
		throw new ApiResponseError({
			status: 429,
			code: "TOO_MANY_REQUESTS",
			message: "This alias changed too often today. Try again tomorrow.",
		});
	});
	await act(async () => button("Save alias").click());
	await settle();
	const alert = Array.from(
		browser.document.querySelectorAll('[role="alert"]'),
	).map((node) => node.textContent);
	expect(alert).toContain(
		"This alias changed too often today. Try again tomorrow.",
	);
	expect(browser.document.body.textContent).not.toContain("TOO_MANY_REQUESTS");
	expect(inputs.get("inbound-email-alias")?.value).toBe("invoices");
});

test("read-only viewers can copy addresses but cannot edit an alias", async () => {
	response = { ...response, alias: "invoices", active: false };
	const writeText = mock(async (_value: string) => {});
	Object.defineProperty(browser.navigator, "clipboard", {
		configurable: true,
		value: { writeText },
	});
	await render("event", false);
	expect(inputs.get("inbound-email-alias")?.disabled).toBe(true);
	expect(browser.document.body.textContent).not.toContain("Save alias");
	expect(browser.document.body.textContent).not.toContain("Remove alias");
	expect(
		browser.document.querySelector(
			'[aria-label="Copy generated email address"]',
		),
	).not.toBeNull();
	await act(async () =>
		document
			.querySelector<HTMLButtonElement>(
				'[aria-label="Copy generated email address"]',
			)
			?.click(),
	);
	expect(writeText).toHaveBeenLastCalledWith("m-generated@mail.example.com");
	await act(async () =>
		document
			.querySelector<HTMLButtonElement>('[aria-label="Copy email alias"]')
			?.click(),
	);
	expect(writeText).toHaveBeenLastCalledWith("invoices@mail.example.com");
	expect(browser.document.body.textContent).toContain("event is inactive");
});
