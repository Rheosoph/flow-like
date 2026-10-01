import { afterAll, afterEach, describe, expect, test } from "bun:test";
import type { ReactNode } from "react";
import {
	allByRole,
	byRole,
	byText,
	click,
	clickByText,
	dropFiles,
	inPortal,
	installDom,
	queryByRole,
	settle,
	typeInto,
} from "../testing/dom-harness";
import type { DevicesT, HubFreshnessState } from "./area-context";
import type { ConfirmOptions, ConfirmResult } from "./confirm-sheet";

const dom = installDom();
const { getI18n } = await import("@flow-like/locales");
const { act, useEffect } = await import("react");
const area = await import("./area-context");
const chips = await import("./status-chip");
const { PresenceGlyph } = await import("./presence-glyph");
const { SeverityWord } = await import("./severity-word");
const stamp = await import("./freshness-stamp");
const gates = await import("./gate-notice");
const { ConsequencePreview } = await import("./consequence-preview");
const confirm = await import("./confirm-sheet");
const { InlineConfirm } = await import("./inline-confirm");
const { InlineResult } = await import("./inline-result");
const { StateView, LockedDataBanner } = await import("./state-view");
const { Block, PageHeader, ObjectHeader } = await import("./block");
const table = await import("./dv-table");
const { KeyValueList, KvRow } = await import("./key-value-list");
const { IdRef } = await import("./id-ref");
const { Banner } = await import("./banner");
const { Segmented, FilterChip } = await import("./segmented");
const { UnderlineTabs } = await import("./underline-tabs");
const { TabsContent } = await import("../../../ui/tabs");
const forms = await import("./form-fields");
const { DvSheet } = await import("./dv-sheet");
const { Kbd } = await import("./kbd");
const { DvButton } = await import("./dv-button");
const labels = await import("../copy/enum-labels");

const NOW_S = 1_790_769_600;

function At({
	children,
	hub,
}: Readonly<{ children: ReactNode; hub?: HubFreshnessState }>) {
	return (
		<area.AreaNowContext.Provider value={NOW_S * 1000}>
			<area.HubFreshnessContext.Provider value={hub ?? { failing: false }}>
				{children}
			</area.HubFreshnessContext.Provider>
		</area.AreaNowContext.Provider>
	);
}

const t = getI18n().getFixedT("en", "devices") as DevicesT;
const BASE_ROWS = {
	what: "Its 2 instances stop.",
	who: "The service page stops answering.",
	when: "Immediately.",
	undo: { reversible: true, text: "Start runs the same settings again." },
} as const;

afterEach(dom.cleanup);
afterAll(dom.restore);

describe("status chips", () => {
	test("critical health is crimson, octagon, square-cornered and worded (R14)", async () => {
		const { container } = await dom.render(
			<chips.HealthChip level="critical" count={2} />,
		);
		const chip = container.querySelector("[data-health=critical]");
		expect(chip?.getAttribute("data-tone")).toBe("critical");
		expect(chip?.className).toContain("rounded-sm");
		expect(chip?.className).toContain("text-critical");
		expect(chip?.className).not.toContain("primary");
		expect(chip?.querySelector("svg.lucide-octagon-x")).not.toBeNull();
		expect(chip?.textContent).toBe("Critical · 2");
	});

	test("every chip family renders each state with words", async () => {
		const health = [
			"critical",
			"attention",
			"healthy",
			"unknown",
			"revoked",
		] as const;
		const presence = ["online", "late", "offline", "never", "revoked"] as const;
		const keys = [
			"live",
			"reconnecting",
			"unlocking",
			"unlocked",
			"held_elsewhere",
			"blocked",
			"locked",
			"stale",
			"none",
		] as const;
		const backups = [
			"never",
			"in_sync",
			"upload_pending",
			"local_changes",
			"hub_newer",
		] as const;
		const relationships = [
			"owner",
			"shared",
			"cloud_approval",
			"unknown",
		] as const;
		const convs = [
			"converged",
			"converging",
			"crash_looping",
			"stopped_by_user",
			"update_in_progress",
			"failed_stopped",
			"unknown",
		] as const;
		const { container } = await dom.render(
			<At>
				{health.map((level) => (
					<chips.HealthChip key={`h-${level}`} level={level} />
				))}
				{presence.map((kind) => (
					<chips.PresenceChip
						key={`p-${kind}`}
						kind={kind}
						since={NOW_S - 41}
					/>
				))}
				{keys.map((state) => (
					<chips.KeyChip
						key={`k-${state}`}
						state={state}
						renewsAt={NOW_S + 244}
					/>
				))}
				{backups.map((backup) => (
					<chips.SafetyChip key={`s-${backup}`} backup={backup} revision={3} />
				))}
				{relationships.map((relationship) => (
					<chips.RelationshipChip
						key={`r-${relationship}`}
						relationship={relationship}
						ownerName="Mira Novak"
					/>
				))}
				{convs.map((conv) => (
					<chips.ConvergenceChip key={`c-${conv}`} conv={conv} />
				))}
			</At>,
		);
		const texts = Array.from(
			container.querySelectorAll("[data-slot=badge]"),
			(el) => el.textContent ?? "",
		);
		expect(texts).toHaveLength(
			health.length +
				presence.length +
				keys.length +
				backups.length +
				relationships.length +
				convs.length,
		);
		for (const text of texts) expect(text.trim().length).toBeGreaterThan(1);
		expect(texts).toContain("Live · direct · renews in 4:04");
		expect(texts).toContain("Backed up (v3)");
		expect(texts).toContain("Shared by Mira Novak");
		expect(texts).toContain("As requested");
		expect(texts.some((text) => text.startsWith("Offline since "))).toBe(true);
		const unknown = container.querySelector("[data-health=unknown]");
		expect(unknown?.className).toContain("border-dashed");
	});

	test("presence glyphs are labelled shapes, decorative inside chips", async () => {
		const { container } = await dom.render(
			<At>
				<PresenceGlyph kind="offline" />
				<chips.PresenceChip kind="online" since={NOW_S - 5} />
			</At>,
		);
		expect(byRole("img", "Offline").getAttribute("data-presence")).toBe(
			"offline",
		);
		const inChip = container.querySelector("[data-slot=badge] [data-presence]");
		expect(inChip?.getAttribute("aria-hidden")).toBe("true");
	});

	test("relationship none renders nothing; unsafe storage wins over backup", async () => {
		const { container } = await dom.render(
			<>
				<chips.RelationshipChip relationship="none" />
				<chips.SafetyChip backup="in_sync" revision={2} atRisk />
			</>,
		);
		expect(container.querySelector("[data-relationship]")).toBeNull();
		expect(container.textContent).toBe("Browser may delete keys");
	});

	test("severity words map to tone and icon", async () => {
		const { container } = await dom.render(
			<>
				<SeverityWord severity="critical" />
				<SeverityWord severity="notice" variant="label" />
				<SeverityWord severity="warning" iconOnly />
			</>,
		);
		expect(
			container.querySelector("[data-severity=critical]")?.className,
		).toContain("text-critical");
		expect(
			container.querySelector("[data-severity=notice]")?.className,
		).toContain("text-info");
		expect(byRole("img", "Warning")).toBeDefined();
	});
});

describe("freshness stamp", () => {
	const ages = [
		"live",
		"current",
		"delayed",
		"lastknown",
		"snapshot",
		"locked",
		"notloaded",
		"noaccess",
		"unsupported",
		"error",
	] as const;

	test("every age state renders its glyph and an absolute-time hover", async () => {
		const { container } = await dom.render(
			<At>
				{ages.map((age) => (
					<stamp.FreshnessStamp
						key={age}
						source="snap"
						age={age}
						observedAt={NOW_S - 13}
					/>
				))}
			</At>,
		);
		const stamps = container.querySelectorAll("[data-stamp]");
		expect(stamps).toHaveLength(ages.length);
		for (const age of ages) {
			const el = container.querySelector(`[data-age=${age}]`);
			expect(el?.querySelector(`[data-glyph=${age}]`)).not.toBeNull();
			expect(el?.getAttribute("title")).toContain(
				"Only keys on this computer can read it",
			);
		}
		const delayed = container.querySelector("[data-age=delayed]");
		expect(delayed?.textContent).toContain("Delayed · ");
		expect(delayed?.getAttribute("title")).toMatch(/^Delayed · Read /);
		expect(delayed?.getAttribute("title")).toContain("at least every 60 s");
	});

	test("hub stamps follow a failing hub unless noFail (R5)", async () => {
		const hub = {
			failing: true,
			dataFrom: NOW_S - 30,
			retryAt: NOW_S + 27,
			reason: "The request timed out after 15 s.",
		};
		const { container } = await dom.render(
			<At hub={hub}>
				<stamp.FreshnessStamp
					source="hub"
					age="current"
					observedAt={NOW_S - 30}
				/>
				<stamp.FreshnessStamp
					source="hub"
					age="current"
					observedAt={NOW_S - 30}
					noFail
				/>
				<stamp.FreshnessStamp source="live" age="live" observedAt={NOW_S - 4} />
			</At>,
		);
		const stamps = Array.from(container.querySelectorAll("[data-stamp]"));
		expect(stamps.map((el) => el.getAttribute("data-age"))).toEqual([
			"error",
			"current",
			"live",
		]);
		const [failing, kept] = stamps as [Element, Element, Element];
		expect(failing.textContent).toContain("couldn't refresh · data from ");
		expect(failing.textContent).toEndWith(" · retry in 0:27");
		expect(failing.getAttribute("title")).toContain("timed out");
		expect(failing.className).toContain("border-critical-line");
		expect(kept.textContent).toContain("checked ");
	});

	test("compact hides the source word visually but keeps it for screen readers", async () => {
		const { container } = await dom.render(
			<At>
				<stamp.FreshnessStamp source="local" age="current" compact />
			</At>,
		);
		const word = byText("This computer", container);
		expect(word.className).toContain("sr-only");
	});

	test("an age without a time reads as its word, never a dangling separator", async () => {
		const { container } = await dom.render(
			<At>
				<stamp.FreshnessStamp source="snap" age="current" />
				<stamp.FreshnessStamp source="snap" age="delayed" />
				<stamp.FreshnessStamp source="snap" age="lastknown" />
				<stamp.FreshnessStamp source="saved" age="snapshot" />
				<stamp.FreshnessStamp source="hub" age="current" />
				<stamp.FreshnessStamp source="local" age="current" />
			</At>,
		);
		const texts = Array.from(
			container.querySelectorAll("[data-stamp] [data-glyph] + span"),
			(el) => el.textContent,
		);
		expect(texts).toEqual([
			"Current",
			"Delayed",
			"Last known",
			"Snapshot",
			"Current",
			"stored here",
		]);
	});

	test("baseSource and sameSource pick and compare row stamps", () => {
		const hub = { source: "hub", age: "current" } as const;
		const live = { source: "live", age: "live" } as const;
		expect(stamp.baseSource([hub, live, hub, null])).toEqual(hub);
		expect(stamp.baseSource([hub, live])).toBeNull();
		expect(stamp.sameSource(hub, { ...hub })).toBe(true);
		expect(stamp.sameSource(hub, live)).toBe(false);
	});
});

describe("gates", () => {
	test("gate notice kinds carry their icon and text", async () => {
		const { container } = await dom.render(
			<gates.GateNotice
				kind="locked"
				title="Unlock lab-gpu-02 to see its services."
				text="The hub can't read them."
				have="Your access: View status"
				actions={<DvButton size="sm">Unlock…</DvButton>}
			/>,
		);
		const notice = container.querySelector("[data-gate=locked]");
		expect(notice?.className).toContain("border-locked-line");
		expect(notice?.querySelector("svg.lucide-lock")).not.toBeNull();
		expect(notice?.textContent).toContain("The hub can't read them.");
	});

	test("gated action stays visible, aria-disabled, with a described visible reason (R7)", async () => {
		let clicks = 0;
		await dom.render(
			<>
				<gates.GatedAction
					gate={{ kind: "busy", reason: "Wait for the update to finish." }}
				>
					<DvButton onClick={() => clicks++}>Reboot device…</DvButton>
				</gates.GatedAction>
				<gates.GatedAction gate={null}>
					<DvButton onClick={() => clicks++}>Refresh</DvButton>
				</gates.GatedAction>
			</>,
		);
		const gated = byRole("button", "Reboot device…");
		expect(gated.getAttribute("aria-disabled")).toBe("true");
		const reasonId = gated.getAttribute("aria-describedby") ?? "";
		expect(document.getElementById(reasonId)?.textContent).toBe(
			"Wait for the update to finish.",
		);
		await click(gated);
		expect(clicks).toBe(0);
		await click(byRole("button", "Refresh"));
		expect(clicks).toBe(1);
	});
});

describe("consequence preview", () => {
	test("rows render in the fixed order; permanent undo is tinted", async () => {
		const { container } = await dom.render(
			<ConsequencePreview
				rows={{
					first: "Stop it first.",
					undo: { reversible: false, text: "Deploy again under a new ID." },
					when: "Immediately.",
					stays: "Its data stays on the device.",
					who: "Cloud access stays.",
					what: "support-bot is removed.",
				}}
			/>,
		);
		const kinds = Array.from(container.querySelectorAll("[data-kind]"), (el) =>
			el.getAttribute("data-kind"),
		);
		expect(kinds).toEqual(["what", "who", "stays", "when", "undo", "first"]);
		const undo = container.querySelector("[data-kind=undo]");
		expect(undo?.getAttribute("data-reversible")).toBe("false");
		expect(undo?.querySelector("dd")?.className).toContain("bg-critical-bg");
		expect(undo?.textContent).toContain("No, this is permanent.");
	});
});

describe("confirm", () => {
	type ConfirmFn = ReturnType<typeof confirm.useConfirm>;

	function Asker({ onReady }: Readonly<{ onReady(ask: ConfirmFn): void }>) {
		const ask = confirm.useConfirm();
		useEffect(() => onReady(ask), [ask, onReady]);
		return null;
	}

	function Opener({
		options,
		onResult,
	}: Readonly<{
		options: ConfirmOptions;
		onResult(result: ConfirmResult): void;
	}>) {
		const ask = confirm.useConfirm();
		useEffect(() => {
			void ask(options).then(onResult);
		}, [ask, options, onResult]);
		return null;
	}

	async function open(options: ConfirmOptions) {
		const results: ConfirmResult[] = [];
		await dom.render(
			<confirm.ConfirmProvider>
				<Opener options={options} onResult={(result) => results.push(result)} />
			</confirm.ConfirmProvider>,
		);
		await settle();
		return { results, sheet: inPortal("alertdialog") };
	}

	test("none: confirm resolves ok, double submit runs once", async () => {
		let runs = 0;
		let release: () => void = () => {};
		const { results, sheet } = await open({
			title: "Stop support-bot?",
			rows: BASE_ROWS,
			confirmLabel: "Stop support-bot",
			tone: "danger",
			onConfirm: () =>
				new Promise<void>((resolve) => {
					runs++;
					release = resolve;
				}),
		});
		expect(byText("Before this runs", sheet)).toBeDefined();
		const button = byRole("button", "Stop support-bot", sheet);
		expect(button.getAttribute("data-variant")).toBe("danger");
		await click(button);
		await click(button);
		expect(runs).toBe(1);
		expect(button.getAttribute("aria-busy")).toBe("true");
		release();
		await settle();
		expect(results).toEqual([{ ok: true, reason: undefined, note: undefined }]);
		expect(queryByRole("alertdialog")).toBeNull();
	});

	test("check: confirm stays disabled until ticked", async () => {
		const { results, sheet } = await open({
			title: "Reboot edge-berlin-01?",
			rows: BASE_ROWS,
			strength: "check",
			checkLabel: "Interrupt the running services now",
			confirmLabel: "Reboot edge-berlin-01",
		});
		const button = byRole("button", "Reboot edge-berlin-01", sheet);
		expect(button.getAttribute("aria-disabled")).toBe("true");
		await click(button);
		expect(results).toHaveLength(0);
		await click(
			byRole("checkbox", "Interrupt the running services now", sheet),
		);
		expect(button.getAttribute("aria-disabled")).toBeNull();
		await click(button);
		await settle();
		expect(results[0]?.ok).toBe(true);
	});

	test("typed: enables only on an exact match", async () => {
		const { results, sheet } = await open({
			title: "Remove support-bot?",
			rows: BASE_ROWS,
			strength: "typed",
			typed: "support-bot",
			confirmLabel: "Remove support-bot",
			tone: "danger",
		});
		const input = byRole("textbox", /Type support-bot to confirm/, sheet);
		const button = byRole("button", "Remove support-bot", sheet);
		await typeInto(input, "support-bo");
		expect(button.getAttribute("aria-disabled")).toBe("true");
		await typeInto(input, "Support-bot");
		expect(button.getAttribute("aria-disabled")).toBe("true");
		await typeInto(input, "support-bot");
		expect(button.getAttribute("aria-disabled")).toBeNull();
		await click(button);
		await settle();
		expect(results[0]?.ok).toBe(true);
	});

	test("reason: needs a reason (and the acknowledgement when attempted)", async () => {
		const { results, sheet } = await open({
			title: "Discard this change?",
			rows: BASE_ROWS,
			strength: "reason",
			reasons: [
				{ value: "duplicate", label: "It's a duplicate" },
				{ value: "wrong", label: "The data is wrong" },
			],
			requireCheck: true,
			checkLabel: "It may already have reached the cloud",
			confirmLabel: "Discard this change",
		});
		const button = byRole("button", "Discard this change", sheet);
		await click(byRole("radio", "The data is wrong", sheet));
		expect(button.getAttribute("aria-disabled")).toBe("true");
		await click(
			byRole("checkbox", "It may already have reached the cloud", sheet),
		);
		await typeInto(byRole("textbox", "Note (optional)", sheet), "  bad row ");
		await click(button);
		await settle();
		expect(results[0]).toEqual({ ok: true, reason: "wrong", note: "bad row" });
	});

	test("review: two steps, then the typed name", async () => {
		const { results, sheet } = await open({
			title: "Revoke edge-berlin-01?",
			rows: BASE_ROWS,
			strength: "review",
			typed: "edge-berlin-01",
			confirmLabel: "Revoke edge-berlin-01",
			tone: "danger",
		});
		expect(byText("Step 1 of 2 · Review", sheet)).toBeDefined();
		expect(queryByRole("button", "Revoke edge-berlin-01", sheet)).toBeNull();
		await click(byRole("button", "Continue", sheet));
		expect(byText("Step 2 of 2 · Confirm", sheet)).toBeDefined();
		await typeInto(
			byRole("textbox", /Type edge-berlin-01/, sheet),
			"edge-berlin-01",
		);
		await click(byRole("button", "Revoke edge-berlin-01", sheet));
		await settle();
		expect(results[0]?.ok).toBe(true);
	});

	test("cancel resolves not ok; a failing action stays open with the error", async () => {
		const { results, sheet } = await open({
			title: "Stop support-bot?",
			rows: BASE_ROWS,
			confirmLabel: "Stop support-bot",
			onConfirm: () => {
				throw new Error("The device didn't reply.");
			},
		});
		await click(byRole("button", "Stop support-bot", sheet));
		await settle();
		expect(byRole("alert", /The device didn't reply\./)).toBeDefined();
		expect(results).toHaveLength(0);
		await click(byRole("button", "Cancel", sheet));
		await settle();
		expect(results).toEqual([{ ok: false }]);
	});

	test("typed: a name with markup characters is shown exactly as it must be typed", async () => {
		const { sheet } = await open({
			title: "Remove this service?",
			rows: BASE_ROWS,
			strength: "typed",
			typed: "a<b>c & <1>d</1>",
			confirmLabel: "Remove",
		});
		const input = byRole("textbox", "Type a<b>c & <1>d</1> to confirm", sheet);
		const button = byRole("button", "Remove", sheet);
		await typeInto(input, "a<b>c & <1>d</1>");
		expect(button.getAttribute("aria-disabled")).toBeNull();
	});

	async function provider() {
		const asker: { ask?: ConfirmFn } = {};
		const mounted = await dom.render(
			<confirm.ConfirmProvider>
				<Asker
					onReady={(fn) => {
						asker.ask = fn;
					}}
				/>
			</confirm.ConfirmProvider>,
		);
		const request = async (options: ConfirmOptions) => {
			const results: ConfirmResult[] = [];
			await act(async () => {
				void asker.ask?.(options).then((result) => results.push(result));
			});
			await settle();
			return results;
		};
		return { request, unmount: mounted.unmount };
	}

	test("a running confirm keeps the sheet; a second request is refused, never confirmed by the first", async () => {
		const { request } = await provider();
		let release: () => void = () => {};
		const first = await request({
			title: "Stop support-bot?",
			rows: BASE_ROWS,
			confirmLabel: "Stop support-bot",
			onConfirm: () =>
				new Promise<void>((resolve) => {
					release = resolve;
				}),
		});
		await click(byRole("button", "Stop support-bot", inPortal("alertdialog")));
		const second = await request({
			title: "Revoke edge-berlin-01?",
			rows: BASE_ROWS,
			confirmLabel: "Revoke edge-berlin-01",
			tone: "danger",
		});
		expect(second).toEqual([{ ok: false }]);
		expect(queryByRole("button", "Revoke edge-berlin-01")).toBeNull();
		expect(
			byRole("button", "Stop support-bot", inPortal("alertdialog")),
		).toBeDefined();
		release();
		await settle();
		expect(first).toEqual([{ ok: true, reason: undefined, note: undefined }]);
		expect(second).toEqual([{ ok: false }]);
		expect(queryByRole("alertdialog")).toBeNull();
	});

	test("an idle confirm is replaced and resolves not ok; unmounting settles the open one", async () => {
		const { request, unmount } = await provider();
		const first = await request({
			title: "Stop support-bot?",
			rows: BASE_ROWS,
			confirmLabel: "Stop support-bot",
		});
		const second = await request({
			title: "Restart support-bot?",
			rows: BASE_ROWS,
			confirmLabel: "Restart support-bot",
		});
		expect(first).toEqual([{ ok: false }]);
		expect(second).toHaveLength(0);
		expect(
			byRole("button", "Restart support-bot", inPortal("alertdialog")),
		).toBeDefined();
		await unmount();
		await settle();
		expect(second).toEqual([{ ok: false }]);
	});
});

describe("inline confirm and result", () => {
	test("inline confirm runs once and reports an inline error", async () => {
		let runs = 0;
		await dom.render(
			<InlineConfirm
				label="Confirm restart"
				title="Restart support-bot?"
				sub="Support Portal · edge-berlin-01"
				rows={BASE_ROWS}
				confirmLabel="Restart support-bot"
				onConfirm={async () => {
					runs++;
					throw new Error("Refused.");
				}}
				onCancel={() => {}}
			/>,
		);
		const region = byRole("region", "Confirm restart");
		await click(byRole("button", "Restart support-bot", region));
		await settle();
		expect(runs).toBe(1);
		expect(byRole("alert", /Refused\./)).toBeDefined();
	});

	test("inline results carry tone, icon and dismiss", async () => {
		let dismissed = 0;
		const { container } = await dom.render(
			<>
				<InlineResult tone="good" onDismiss={() => dismissed++}>
					Stopped at 14:02:11.
				</InlineResult>
				<InlineResult tone="info">
					Requested. Waiting for the device…
				</InlineResult>
				<InlineResult tone="unknown">No reply received.</InlineResult>
			</>,
		);
		expect(
			container.querySelector("[data-result=info] svg")?.getAttribute("class"),
		).toContain("animate-spin");
		expect(
			container.querySelector("[data-result=unknown]")?.className,
		).toContain("border-dashed");
		await clickByText("Dismiss");
		expect(dismissed).toBe(1);
	});
});

describe("state view", () => {
	test("each kind is distinct and never reads as empty (R6)", async () => {
		const kinds = [
			"empty",
			"notloaded",
			"never",
			"locked",
			"noaccess",
			"unsupported",
			"error",
			"gate",
		] as const;
		const { container } = await dom.render(
			<>
				{kinds.map((kind) => (
					<StateView key={kind} kind={kind} />
				))}
				<StateView kind="loading" />
			</>,
		);
		const titles = kinds.map(
			(kind) =>
				container.querySelector(`[data-kind=${kind}] p`)?.textContent ?? "",
		);
		expect(new Set(titles).size).toBe(kinds.length);
		expect(container.querySelector("[data-kind=error]")?.className).toContain(
			"border-critical-line",
		);
		expect(byRole("status", "Loading…").getAttribute("aria-busy")).toBe("true");
	});

	test("locked-data banner names the read time", async () => {
		await dom.render(
			<At>
				<LockedDataBanner readAt={NOW_S - 10} />
			</At>,
		);
		expect(byRole("status").textContent).toMatch(
			/^Locked\. Showing what was read at .+\.$/,
		);
	});

	test("banners announce critical conditions", async () => {
		await dom.render(
			<Banner tone="critical" title="Hub unreachable">
				Retrying in 30 s.
			</Banner>,
		);
		expect(byRole("alert").textContent).toContain("Hub unreachable");
	});
});

describe("block and headers", () => {
	test("block head carries title, count and stamp slot", async () => {
		await dom.render(
			<Block
				title="Needs you"
				count={15}
				stamp={<span>stamp</span>}
				foot="foot"
			>
				body
			</Block>,
		);
		const region = byRole("region", /Needs you/);
		expect(region.textContent).toContain("15");
		expect(region.textContent).toContain("stamp");
		expect(region.querySelector("footer")?.textContent).toBe("foot");
	});

	test("page header: crumbs navigate client-side, one primary (R2)", async () => {
		let went = 0;
		await dom.render(
			<PageHeader
				crumbs={[
					{
						label: "Devices",
						href: "/settings/devices",
						onNavigate: () => went++,
					},
					{ label: "Fleet overview" },
				]}
				title="Fleet overview"
				actions={
					<>
						<DvButton>Refresh</DvButton>
						<DvButton variant="primary">Set up a device</DvButton>
					</>
				}
			/>,
		);
		await clickByText("Devices");
		expect(went).toBe(1);
		expect(byRole("heading", "Fleet overview")).toBeDefined();
		expect(document.querySelectorAll("[data-dv-primary]")).toHaveLength(1);
	});

	test("a crumb without an href is a keyboard-reachable button; one without a target is plain text", async () => {
		let went = 0;
		const { container } = await dom.render(
			<PageHeader
				crumbs={[
					{ label: "Devices", onNavigate: () => went++ },
					{ label: "edge-berlin-01" },
					{ label: "Certificates" },
				]}
				title="Certificates"
			/>,
		);
		await click(byRole("button", "Devices"));
		expect(went).toBe(1);
		expect(container.querySelectorAll("nav a")).toHaveLength(0);
		expect(byText("edge-berlin-01", container).tagName).toBe("SPAN");
	});

	test("object header renders a mono name and facts", async () => {
		await dom.render(
			<ObjectHeader
				name="edge-berlin-01"
				facts={[{ id: "agent", label: "Agent", value: "0.9.4" }]}
			/>,
		);
		expect(byText("edge-berlin-01").className).toContain("font-mono");
		expect(byText("Agent")).toBeDefined();
	});
});

describe("buttons", () => {
	test("variants never hover coral; busy swaps the icon and blocks clicks", async () => {
		let clicks = 0;
		const { container } = await dom.render(
			<>
				<DvButton>Default</DvButton>
				<DvButton variant="ghost">Ghost</DvButton>
				<DvButton variant="danger-ghost">Revoke…</DvButton>
				<DvButton busy onClick={() => clicks++}>
					Saving
				</DvButton>
			</>,
		);
		for (const button of Array.from(container.querySelectorAll("button"))) {
			expect(button.className).not.toMatch(/hover:bg-(accent|primary)\b/);
			expect(button.className).not.toMatch(/\bshadow-(xs|sm|md|lg)\b/);
		}
		const busy = byRole("button", "Saving");
		expect(busy.getAttribute("aria-busy")).toBe("true");
		expect(busy.querySelector("svg.animate-spin")).not.toBeNull();
		await click(busy);
		expect(clicks).toBe(0);
	});
});

describe("data table", () => {
	test("cells carry data-label for stacked cards; sort header exposes aria-sort", async () => {
		let sorted = 0;
		await dom.render(
			<table.DvTable
				label="Devices"
				cols={["40%", "auto"]}
				head={
					<tr>
						<table.SortHeader sort="descending" onSort={() => sorted++}>
							Health
						</table.SortHeader>
						<table.Th>Device</table.Th>
					</tr>
				}
			>
				<table.GroupRow colSpan={2}>edge-berlin-01</table.GroupRow>
				<table.Tr>
					<table.Td label="Health">Healthy</table.Td>
					<table.Td label="Device" kind="mono">
						edge-berlin-01
					</table.Td>
				</table.Tr>
			</table.DvTable>,
		);
		const grid = byRole("table", "Devices");
		expect(grid.getAttribute("data-stack")).toBe("900");
		expect(grid.className).toContain("table-fixed");
		expect(grid.querySelectorAll("col")).toHaveLength(2);
		const cellLabels = allByRole("cell").map((cell) =>
			cell.getAttribute("data-label"),
		);
		expect(cellLabels).toEqual([null, "Health", "Device"]);
		expect(byRole("columnheader", /Health/).getAttribute("aria-sort")).toBe(
			"descending",
		);
		await clickByText("Health");
		expect(sorted).toBe(1);
	});
});

describe("key-value list and id ref", () => {
	test("rows render term, value and provenance", async () => {
		const { container } = await dom.render(
			<KeyValueList>
				<KvRow label="Platform" provenance="setup record">
					Linux
				</KvRow>
			</KeyValueList>,
		);
		expect(container.querySelector("dt")?.textContent).toBe("Platform");
		expect(container.querySelector("[data-provenance]")?.textContent).toBe(
			"setup record",
		);
	});

	test("id ref shows 8 chars, reveals the full value and copies it", async () => {
		const id = "5b794764-6afc-4ac9-89c2-c6d9eb91fd42";
		await dom.render(
			<IdRef id={id} label="Device ID" copyLabel="Copy device ID" />,
		);
		const value = byRole("button", "5b794764");
		expect(value.getAttribute("aria-expanded")).toBe("false");
		await click(value);
		expect(value.textContent).toBe(id);
		await click(byRole("button", "Copy device ID"));
		await settle();
		expect(dom.clipboard).toEqual([id]);
		expect(byRole("button", "Copied")).toBeDefined();
	});

	test("fingerprints group in blocks of 4", async () => {
		await dom.render(<IdRef id="3f2a91c07b4ed158aa00bb11" group4 />);
		expect(byRole("button", "3F2A 91C0 7B4E D158 …")).toBeDefined();
	});
});

describe("tabs, segmented, filter chips, kbd", () => {
	test("segmented and filter chips toggle aria-pressed", async () => {
		const picked: string[] = [];
		const toggled: boolean[] = [];
		await dom.render(
			<>
				<Segmented
					label="View"
					value="devices"
					onChange={(value) => picked.push(value)}
					options={[
						{ value: "devices", label: "Devices", count: 7 },
						{ value: "services", label: "Services", count: 5 },
					]}
				/>
				<FilterChip
					pressed={false}
					onPressedChange={(next) => toggled.push(next)}
				>
					Needs attention
				</FilterChip>
				<Kbd>/</Kbd>
			</>,
		);
		expect(byRole("group", "View")).toBeDefined();
		expect(byRole("button", /^Devices/).getAttribute("aria-pressed")).toBe(
			"true",
		);
		await click(byRole("button", /^Services/));
		await click(byRole("button", "Needs attention"));
		expect(picked).toEqual(["services"]);
		expect(toggled).toEqual([true]);
	});

	test("underline tabs are Radix tabs with count badges", async () => {
		const selected: string[] = [];
		await dom.render(
			<UnderlineTabs
				label="Device sections"
				value="overview"
				onValueChange={(next) => selected.push(next)}
				tabs={[
					{
						value: "overview",
						label: "Overview",
						count: { count: 3, tone: "warning", label: "3 need attention" },
					},
					{
						value: "certificates",
						label: "Certificates",
						count: { count: 1, tone: "critical" },
					},
				]}
			>
				<TabsContent value="overview">Overview panel</TabsContent>
			</UnderlineTabs>,
		);
		expect(byRole("tablist", "Device sections")).toBeDefined();
		const tabs = allByRole("tab");
		expect(tabs.map((tab) => tab.getAttribute("aria-selected"))).toEqual([
			"true",
			"false",
		]);
		const [overview, certificates] = tabs as [HTMLElement, HTMLElement];
		expect(overview.querySelector("[aria-label]")).toBeNull();
		expect(overview.querySelector(".sr-only")?.textContent).toBe(
			"3 need attention",
		);
		expect(
			overview.querySelector("[data-count-tone] [aria-hidden]")?.textContent,
		).toBe("3");
		expect(certificates.textContent).toBe("Certificates1");
		expect(
			document.querySelector("[data-count-tone=critical]")?.className,
		).toContain("rounded-sm");
		expect(byRole("tabpanel").textContent).toBe("Overview panel");
		await click(tabs[1] as HTMLElement);
		await settle();
		expect(selected).toEqual(["certificates"]);
	});
});

describe("form fields", () => {
	test("field wires label, hint and error", async () => {
		await dom.render(
			<forms.Field
				id="f-port"
				label="Port"
				hint="1–65535"
				error="Port 80 is used."
			>
				<forms.DvInput numeric defaultValue="80" />
			</forms.Field>,
		);
		const input = byRole("textbox", "Port");
		expect(input.getAttribute("aria-invalid")).toBe("true");
		expect(input.getAttribute("aria-describedby")).toBe(
			"f-port-error f-port-hint",
		);
	});

	test("secret input: show/hide and a UTF-8 byte count", async () => {
		const values: string[] = [];
		const { rerender } = await dom.render(
			<forms.SecretInput
				id="f-pw"
				aria-label="Device password"
				value=""
				onValueChange={(next) => values.push(next)}
				minBytes={12}
				maxBytes={4096}
			/>,
		);
		const input = document.getElementById("f-pw") as HTMLInputElement;
		expect(input.type).toBe("password");
		await typeInto(input, "pässwörd");
		expect(values.at(-1)).toBe("pässwörd");
		await rerender(
			<forms.SecretInput
				id="f-pw"
				aria-label="Device password"
				value="pässwörd"
				onValueChange={(next) => values.push(next)}
				minBytes={12}
				maxBytes={4096}
			/>,
		);
		const count = document.querySelector("[data-bytes]");
		expect(count?.getAttribute("data-bytes")).toBe("10");
		expect(count?.textContent).toMatch(/^10 bytes · 12.4,?096 bytes$/);
		expect(count?.className).toContain("text-critical");
		await click(byRole("button", "Show password"));
		expect(input.type).toBe("text");
		expect(byRole("button", "Hide password").getAttribute("aria-pressed")).toBe(
			"true",
		);
	});

	test("list editor adds and removes rows; choice cards and checks pick values", async () => {
		const lists: string[][] = [];
		const choices: string[] = [];
		const checks: boolean[] = [];
		await dom.render(
			<>
				<forms.ListEditor
					id="f-origins"
					values={["https://a.example"]}
					onChange={(next) => lists.push(next)}
					addLabel="Add origin"
					itemLabel={(index) => `Origin ${index + 1}`}
				/>
				<forms.ChoiceCards
					id="f-plat"
					legend="Platform"
					value={undefined}
					onValueChange={(next) => choices.push(next)}
					options={[
						{ value: "arm", title: "Linux (ARM 64-bit)" },
						{ value: "x64", title: "Linux (Intel/AMD 64-bit)", disabled: true },
					]}
				/>
				<forms.CheckField
					id="f-ack"
					checked={false}
					onCheckedChange={(next) => checks.push(next)}
				>
					I saved the backup file
				</forms.CheckField>
			</>,
		);
		await click(byRole("button", "Add origin"));
		await click(byRole("button", "Remove Origin 1"));
		expect(lists).toEqual([["https://a.example", ""], []]);
		await click(byRole("radio", "Linux (ARM 64-bit)"));
		expect(choices).toEqual(["arm"]);
		await click(byRole("checkbox", "I saved the backup file"));
		expect(checks).toEqual([true]);
	});

	test("switch field: labelled, neutral when on, and its thumb stays visible in dark mode", async () => {
		const flips: boolean[] = [];
		await dom.render(
			<forms.SwitchField
				id="f-backup"
				checked
				onCheckedChange={(next) => flips.push(next)}
			>
				Save an encrypted key backup to my account
			</forms.SwitchField>,
		);
		const toggle = byRole(
			"switch",
			"Save an encrypted key backup to my account",
		);
		expect(toggle.getAttribute("aria-checked")).toBe("true");
		expect(toggle.className).toContain("data-[state=checked]:bg-foreground");
		expect(toggle.className).not.toContain("bg-primary");
		expect(toggle.className).toContain(
			"dark:data-[state=checked]:**:data-[slot=switch-thumb]:bg-background",
		);
		await click(toggle);
		expect(flips).toEqual([false]);
	});

	test("drop zone hands dropped files over", async () => {
		const got: string[] = [];
		await dom.render(
			<forms.DropZone
				id="f-file"
				title="Drop an access request file"
				hint=".json · up to 128 KiB"
				accept=".json"
				onFiles={(files) => got.push(...files.map((file) => file.name))}
			/>,
		);
		const zone = document.querySelector("label[for=f-file]") as HTMLElement;
		await dropFiles(zone, [new File(["{}"], "request.json")]);
		expect(got).toEqual(["request.json"]);
	});
});

describe("sheet", () => {
	test("dv sheet: named dialog, close and back controls, sunken foot", async () => {
		let closed = 0;
		let back = 0;
		await dom.render(
			<DvSheet
				open
				onOpenChange={() => closed++}
				title="Unlock lab-gpu-02"
				sub="Shared by Mira Novak"
				onBack={() => back++}
				footNote="Step 1 of 2"
				foot={<DvButton variant="primary">Continue</DvButton>}
			>
				Body
			</DvSheet>,
		);
		const sheet = inPortal("dialog");
		expect(accessible(sheet)).toBe("Unlock lab-gpu-02");
		await click(byRole("button", "Back", sheet));
		await click(byRole("button", "Close", sheet));
		expect(back).toBe(1);
		expect(closed).toBe(1);
		expect(sheet.querySelectorAll("[data-dv-primary]")).toHaveLength(1);
	});
});

describe("enum labels", () => {
	test("every family maps every value to a non-empty label that is not the wire value", () => {
		for (const family of labels.ENUM_FAMILIES) {
			const rows = labels.enumTable(t, family);
			for (const [value, copy] of Object.entries(rows)) {
				expect(copy.label.trim().length).toBeGreaterThan(0);
				if (/[_-]/.test(value)) expect(copy.label).not.toBe(value);
			}
		}
	});

	test("labels interpolate params and reuse the chip copy", () => {
		expect(labels.enumLabel(t, "capability", "manage_certificates")).toBe(
			"Manage certificates",
		);
		expect(labels.enumLabel(t, "backup", "in_sync", { revision: 3 })).toBe(
			"Backed up (v3)",
		);
		expect(labels.enumLabel(t, "convergence", "crash_looping")).toBe(
			"Crashing",
		);
		expect(
			labels.enumLabel(t, "scopeKind", "project", { name: "Invoice AI" }),
		).toBe("App Invoice AI");
		expect(labels.enumExplain(t, "transport", "webrtc")).toBe(
			"Both are end-to-end encrypted.",
		);
		expect(labels.enumLabel(t, "preset", "device_admin")).toBe("Device admin");
	});

	test("a wire value this client doesn't know reads Unknown instead of throwing (R3)", () => {
		const unknown = (family: string, value: string) =>
			labels.enumLabel(t, family as "capability", value as "status");
		expect(unknown("capability", "manage_firewall")).toBe("Unknown");
		expect(unknown("failureCode", "quota_exceeded")).toBe("Unknown");
		expect(unknown("logStream", "constructor")).toBe("Unknown");
		expect(
			labels.enumExplain(t, "observed", "hibernating" as "running"),
		).toBeUndefined();
	});

	test("a failed update is worded by its failure code; history tiers have labels", () => {
		expect(labels.enumLabel(t, "rollout", "failed")).toBe("Not applied");
		expect(
			labels.enumLabel(t, "rollout", "failed", {
				failureCode: "validation_failed",
			}),
		).toBe("Not applied");
		for (const failureCode of ["rollback_timeout", "rollback_failed"]) {
			expect(labels.enumLabel(t, "rollout", "failed", { failureCode })).toBe(
				"Failed, service stopped",
			);
		}
		expect(labels.enumLabel(t, "telemetryTier", "free")).toBe(
			"Not stored in the cloud",
		);
		expect(
			labels.enumLabel(t, "telemetryTier", "stored", {
				plan: "Pro",
				retention: "30 days",
				size: "5 GB",
			}),
		).toBe("Pro · keeps 30 days, up to 5 GB");
		expect(labels.enumLabel(t, "telemetryTier", "stored")).toBe(
			"Stored in the cloud",
		);
	});
});

function accessible(el: HTMLElement): string {
	const id = el.getAttribute("aria-labelledby") ?? "";
	return document.getElementById(id)?.textContent ?? "";
}
