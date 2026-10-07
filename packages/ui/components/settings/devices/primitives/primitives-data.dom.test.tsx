import {
	afterAll,
	afterEach,
	beforeAll,
	describe,
	expect,
	test,
} from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import type { ComponentProps, ReactNode } from "react";
import { formatTimeOfDay } from "../../../../lib/date";
import {
	allByRole,
	byRole,
	byText,
	click,
	clickByText,
	fire,
	inPortal,
	installDom,
	queryByRole,
	settle,
} from "../testing/dom-harness";
import type {
	AnnunciatorCell,
	AnnunciatorWindow,
	AnnunciatorWindowSpec,
} from "./annunciator";
import type { DevicesT } from "./area-context";
import type { AttentionEntry } from "./attention-list";
import type { FleetRolloutRow } from "./fleet-rollout";
import type { LogRecord } from "./log-viewer";
import type { MatrixCellProps } from "./matrix-cell";
import type { ObservedRun } from "./requested-actual";
import type { ConvergenceChipKind } from "./status-chip";
import type { TimelineRow } from "./timeline";
import type { ChainLink } from "./trust-chain";
import type { WizardFootProps, WizardSummaryItem } from "./wizard";

const dom = installDom();
const { getI18n } = await import("@flow-like/locales");
const area = await import("./area-context");
const { RequestedActual } = await import("./requested-actual");
const { PairedPins } = await import("./paired-pins");
const { RolloutProgress } = await import("./rollout-progress");
const { FleetRollout } = await import("./fleet-rollout");
const { AttentionList } = await import("./attention-list");
const { Annunciator, AnnunciatorWindows, annunciatorSub } = await import(
	"./annunciator"
);
const { monoNames } = await import("./obj-name");
const { DayOf, dayText, untilText } = await import("./day");
const { EventCell, eventIcon } = await import("./event-cell");
const { MetricGrid, Metric } = await import("./metric");
const { Meter, SpendMeter, ProgressBar } = await import("./meter");
const { CoverageLine } = await import("./coverage-line");
const { Headline } = await import("./headline");
const { PersonChip, initialsOf } = await import("./person-chip");
const { CheckinLane } = await import("./checkin-lane");
const { ExpiryRail, ExpiryRailScale } = await import("./expiry-rail");
const { TrustChain } = await import("./trust-chain");
const { Timeline } = await import("./timeline");
const { LogViewer } = await import("./log-viewer");
const { CommandBlock } = await import("./command-block");
const { Checklist } = await import("./checklist");
const wizard = await import("./wizard");
const { TrayItem } = await import("./tray-item");
const { ServiceRow, ServiceRowHead, SERVICE_ROW_COLS } = await import(
	"./service-row"
);
const { DvTable } = await import("./dv-table");
const appChips = await import("./app-chips");
const { HowRunsStrip, ModeExplainer } = await import("./how-runs");
const { MatrixCell } = await import("./matrix-cell");
const { DiffRows } = await import("./diff-rows");
const { DvButton } = await import("./dv-button");
const { Lock, Fingerprint, Laptop, Users } = await import("lucide-react");

const NOW_S = 1_790_769_600;
const DAY = 86_400;
const t = getI18n().getFixedT("en", "devices") as DevicesT;

function At({
	children,
	nowS = NOW_S,
	tech = false,
}: Readonly<{ children: ReactNode; nowS?: number; tech?: boolean }>) {
	return (
		<area.AreaNowContext.Provider value={nowS * 1000}>
			<area.AreaPrefsContext.Provider value={{ showTechnicalKeys: tech }}>
				{children}
			</area.AreaPrefsContext.Provider>
		</area.AreaNowContext.Provider>
	);
}

const texts = (root: ParentNode, selector: string) =>
	Array.from(root.querySelectorAll(selector), (el) =>
		(el.textContent ?? "").replace(/\s+/g, " ").trim(),
	);

const attrs = (root: ParentNode, selector: string, name: string) =>
	Array.from(root.querySelectorAll(selector), (el) => el.getAttribute(name));

function classesOf(el: Element | null | undefined) {
	return el?.className ?? "";
}

afterEach(dom.cleanup);
afterAll(dom.restore);

const PIN_STATES: readonly ObservedRun[] = [
	"running",
	"starting",
	"stopping",
	"backoff",
	"failed",
	"stopped",
	"unknown",
];

const PIN_CONV: Partial<Record<ObservedRun, ConvergenceChipKind>> = {
	backoff: "crash_looping",
	running: "converged",
};

describe("requested → actual and paired pins", () => {
	test("only the actual half takes the tone; the pair has one spoken name", async () => {
		const { container } = await dom.render(
			<At>
				<RequestedActual
					desired="running"
					observed="backoff"
					conv="crash_looping"
					since={NOW_S - 3 * 3600}
					lastKnown
				/>
				<RequestedActual
					versions
					requested="Saved v2"
					actual="device has v1"
					label="Saved v2, device has v1"
				/>
			</At>,
		);
		const pair = byRole(
			"img",
			"Requested Running, actual Restarting after a crash",
		);
		expect(pair.getAttribute("data-tone")).toBe("critical");
		expect(pair.className).toContain("border-critical-line");
		const [requested, , actual] = Array.from(pair.children);
		expect(requested.className).toContain("text-muted-foreground");
		expect(actual.className).toContain("text-critical");
		expect(actual.className).toContain("bg-critical-bg");
		const text = container.textContent ?? "";
		expect(text).toContain("Crashing · ");
		expect(text).toContain(", last known");
		const versions = byRole("img", "Saved v2, device has v1");
		expect(versions.getAttribute("data-tone")).toBe("neutral");
		// In a table cell a long actual state goes under the requested one.
		expect(pair.className).toContain("in-[td]:flex-wrap");
		expect(actual.className).toContain("in-[td]:wrap-break-word");
		expect(pair.className).toContain("rounded-md");
	});

	test("pins draw every actual state and a broken wire for crashes", async () => {
		const { container } = await dom.render(
			<At>
				{PIN_STATES.map((state, index) => {
					return (
						<PairedPins
							key={state}
							desired={state === "stopped" ? "stopped" : "running"}
							observed={state}
							conv={PIN_CONV[state] ?? "converging"}
							title={index === 0}
						/>
					);
				})}
			</At>,
		);
		expect(attrs(container, "[data-act]", "data-act")).toEqual([...PIN_STATES]);
		const pins = Array.from(container.querySelectorAll("[data-act]"));
		for (const pin of pins) expect(pin.querySelector("svg")).not.toBeNull();
		expect(pins[0].getAttribute("title")).toContain("Hollow pin");
		expect(pins[1].getAttribute("title")).toBeNull();
		expect(pins[3].querySelector("[data-wire=crash]")).not.toBeNull();
		expect(pins[0].querySelector("[data-wire=ok]")).not.toBeNull();
		expect(pins[1].querySelector("[data-wire=moving]")).not.toBeNull();
		expect(pins[5].querySelector("rect")).not.toBeNull();
	});
});

function Activating({ nowS }: Readonly<{ nowS: number }>) {
	return (
		<At nowS={nowS}>
			<RolloutProgress
				phase="activating"
				from="settings v11"
				to="settings v12"
				deadlineAt={NOW_S + 90}
				times={{ staged: NOW_S - 240, validated: NOW_S - 180 }}
				instances={{ previous: 1, candidate: 1 }}
				waitingFor="instance #0"
			/>
		</At>
	);
}

const FLEET_PHASES = ["Uploading", "Installing", "Switching over", "Updated"];

function fleetRow(
	id: string,
	state: FleetRolloutRow["state"],
	extra: Partial<FleetRolloutRow> = {},
) {
	const row: FleetRolloutRow = {
		id,
		device: id,
		service: "crm-webhook",
		phases: FLEET_PHASES,
		step: 1,
		state,
		at: NOW_S - 60,
		...extra,
	};
	return row;
}

const FLEET_ROWS = [
	fleetRow("d-done", "done", { version: "v1.5.0" }),
	fleetRow("d-active", "active", { progress: { done: 14, total: 26 } }),
	fleetRow("d-failed", "failed", {
		reason: "the device stopped answering",
		failedWhile: "installing",
	}),
	fleetRow("d-rb", "rolled_back", {
		step: 2,
		reason: "the new version crashed",
		from: "v1.4.0",
	}),
	fleetRow("d-blocked", "blocked"),
	fleetRow("d-skipped", "skipped"),
	fleetRow("d-held", "held"),
	fleetRow("d-not", "not_started"),
	fleetRow("d-wait", "waiting"),
];

const FLEET_SENTENCES = [
	"Updated at ",
	" · v1.5.0",
	"Installing · 14 of 26",
	"Failed while installing: the device stopped answering. Nothing changed on d-failed.",
	"Rolled back at ",
	": the new version crashed. v1.4.0 is running again.",
	"Waiting: unlock d-blocked to continue.",
	"Skipped by you. Nothing changed on d-skipped.",
	"Waiting: d-failed failed. Continue with the rest or stop here.",
	"Not started: you stopped after d-failed failed.",
	"In line after d-not",
];

function UpdateEverywhere() {
	return (
		<At>
			<FleetRollout
				kind="update"
				title="Update Invoice AI everywhere"
				subtitle="v1.4.0 → v1.5.0 · safe update · one device at a time"
				rows={FLEET_ROWS}
				heldBy="d-failed"
				shared={{
					label: "Approving definitions",
					state: "done",
					source: "hub",
				}}
			/>
		</At>
	);
}

const DEPLOY_PHASES = [
	"Creating cloud access",
	"Setting the spending limit",
	"Uploading definitions and packages",
	"Checking events on the device",
	"Installing app version v0.4.0",
	"Creating visitor-checkin",
	"Saving 2 secrets",
	"Starting instance #0",
];

function DeployToTwo() {
	return (
		<At>
			<FleetRollout
				title="Deploy Visitor check-in to 2 devices"
				rows={[
					fleetRow("edge-berlin-01", "active", {
						phases: DEPLOY_PHASES,
						step: 3,
					}),
					fleetRow("studio-mac-mini", "failed", {
						phases: DEPLOY_PHASES,
						step: 2,
						reason: "the device stopped answering",
						actions: <DvButton size="xs">Retry this device</DvButton>,
					}),
				]}
			/>
		</At>
	);
}

function columnPlan(row: Element) {
	const plan = row.className
		.split(" ")
		.find((name) => name.startsWith("grid-cols-["));
	return plan ?? "";
}

describe("rollouts", () => {
	test("single rollout counts down with the injected area clock", async () => {
		const { rerender, container } = await dom.render(
			<Activating nowS={NOW_S} />,
		);
		const countdown = () =>
			container.querySelector("[data-countdown]")?.textContent;
		expect(countdown()).toBe("1:30 left");
		await rerender(<Activating nowS={NOW_S + 10} />);
		expect(countdown()).toBe("1:20 left");
		expect(attrs(container, "ol > li", "data-s")).toEqual([
			"done",
			"done",
			"active",
			"todo",
		]);
		expect(container.textContent).toContain("waiting for instance #0");
		expect(container.textContent).toContain("previous 1 → new 1");
		// Sentences stay in the text face (tabular digits); only step times are mono.
		expect(
			classesOf(container.querySelector("[data-countdown]")),
		).not.toContain("font-mono");
		expect(classesOf(byText("waiting for instance #0", container))).toBe(
			"tabular-nums",
		);
		// Four steps in a phone-wide block wrap 2 × 2.
		expect(classesOf(container.querySelector("ol"))).toContain(
			"@max-[560px]/rollout:grid-cols-2",
		);
		expect(classesOf(container.querySelector("ol")?.parentElement)).toContain(
			"@container/rollout",
		);
	});

	test("end states replace the step labels", async () => {
		const { container } = await dom.render(
			<At>
				<RolloutProgress phase="rolled_back" from="v11" to="v12" />
				<RolloutProgress phase="failed_stopped" from="v11" to="v12" />
				<RolloutProgress
					phase="healthy"
					from="v11"
					to="v12"
					times={{ done: NOW_S - 30 }}
				/>
			</At>,
		);
		const sections = Array.from(container.querySelectorAll("[data-rollout]"));
		expect(sections[0].querySelector("[data-s=fail]")?.textContent).toContain(
			"Rolled back",
		);
		expect(sections[1].textContent).toContain("Failed, service stopped");
		expect(sections[1].querySelector("[data-s=fail]")).not.toBeNull();
		expect(sections[2].querySelectorAll("li[data-s=done]")).toHaveLength(4);
	});

	test("rolling back says what the device is doing, without the activation countdown", async () => {
		const { container } = await dom.render(
			<At>
				<RolloutProgress
					phase="rolling_back"
					from="settings v11"
					to="settings v12"
					deadlineAt={NOW_S - 5}
					instances={{ previous: 1, candidate: 1 }}
				/>
			</At>,
		);
		const text = container.textContent ?? "";
		expect(container.querySelector("[data-countdown]")).toBeNull();
		expect(text).not.toContain("Must stay healthy for");
		expect(text).not.toContain("If it fails");
		expect(text).toContain(
			"The new version didn't stay healthy. The device is restoring settings v11.",
		);
		const active = container.querySelector("li[data-s=active]");
		expect(active?.textContent).toContain("Rolling back");
		expect(active?.getAttribute("aria-current")).toBe("step");
	});

	test("fleet rollout rows stay whole sentences without the optional facts", async () => {
		const bare = (id: string, state: FleetRolloutRow["state"]) => {
			const row: FleetRolloutRow = {
				id,
				device: id,
				phases: ["Uploading", "Starting"],
				step: 0,
				state,
			};
			return row;
		};
		const { container } = await dom.render(
			<At>
				<FleetRollout
					title="Deploy CRM webhook to 4 devices"
					rows={[
						bare("d-a", "failed"),
						bare("d-b", "rolled_back"),
						{ ...bare("d-c", "failed"), reason: "The device refused it." },
						bare("d-d", "held"),
					]}
				/>
			</At>,
		);
		expect(texts(container, "li[data-device] > span:nth-child(4)")).toEqual([
			"Failed while Uploading. Nothing changed on d-a.",
			"Rolled back. The previous version is running again.",
			"Failed while Uploading: The device refused it. Nothing changed on d-c.",
			"Waiting: d-a failed. Continue with the rest or stop here.",
		]);
	});

	test("fleet rollout: rows share one column plan and eight phases fit the step column", async () => {
		const { container } = await dom.render(<DeployToTwo />);
		const rows = Array.from(container.querySelectorAll("li[data-device]"));
		const plans = rows.map(columnPlan);
		expect(plans[0]).toStartWith("grid-cols-[");
		expect(plans[0]).not.toContain("auto");
		expect(plans[1]).toBe(plans[0]);
		const bars = attrs(rows[0], "[data-s]", "class");
		expect(bars).toHaveLength(8);
		for (const bar of bars) {
			expect(bar).toContain("flex-1");
			expect(bar).not.toContain("w-4.5");
		}
	});

	test("fleet rollout renders every APP §7.8 row state", async () => {
		const { container } = await dom.render(<UpdateEverywhere />);
		expect(attrs(container, "li[data-device]", "data-state")).toEqual([
			"done",
			"active",
			"failed",
			"rolled_back",
			"blocked",
			"skipped",
			"held",
			"not_started",
			"waiting",
		]);
		const chip = container.querySelector("[data-fleet-chip]");
		expect(chip?.textContent).toBe("Failed on 2");
		expect(chip?.getAttribute("data-tone")).toBe("warning");
		const rb = container.querySelector("li[data-state=rolled_back]");
		expect(rb?.querySelector("[data-s=fail]")).not.toBeNull();
		expect(container.querySelector("[data-state=pass]")).not.toBeNull();
	});

	test("fleet rollout rows say what happened and what comes next", async () => {
		const { container } = await dom.render(<UpdateEverywhere />);
		const text = container.textContent ?? "";
		for (const sentence of FLEET_SENTENCES) expect(text).toContain(sentence);
	});

	test("fleet chip is critical only when every device failed", async () => {
		const { container } = await dom.render(
			<At>
				<FleetRollout
					title="Deploy CRM webhook to 1 device"
					rows={[
						{
							id: "a",
							device: "a",
							phases: ["Uploading"],
							step: 0,
							state: "failed",
							reason: "x",
						},
					]}
				/>
				<FleetRollout
					title="Deploy CRM webhook to 1 device"
					rows={[
						{
							id: "b",
							device: "b",
							phases: ["Uploading"],
							step: 0,
							state: "done",
							at: NOW_S,
						},
					]}
				/>
			</At>,
		);
		const chips = Array.from(container.querySelectorAll("[data-fleet-chip]"));
		expect(chips[0].getAttribute("data-tone")).toBe("critical");
		expect(chips[1].textContent).toBe("1 of 1 done");
		expect(chips[1].getAttribute("data-tone")).toBe("good");
		expect(container.textContent).toContain(" · b running");
	});
});

const ATTENTION_ITEMS: AttentionEntry[] = [
	{
		id: "a1",
		severity: "critical",
		sentence: "warehouse-pi hasn't checked in since 11:00.",
		conditionKey: "offline_since",
		since: NOW_S - 3 * 3600,
		stamp: { source: "hub", age: "current" },
		action: { label: "Diagnose" },
	},
	{
		id: "a2",
		severity: "warning",
		sentence: "internal-mqtt expires in 5 days.",
		stamp: { source: "hub", age: "current" },
		action: {
			label: "Renew…",
			gate: { kind: "locked", reason: "Unlock edge-berlin-01 first." },
		},
	},
	{
		id: "a3",
		severity: "notice",
		sentence: "No account backup yet.",
		stamp: { source: "local", age: "current" },
		onSnooze: () => {},
		action: { label: "Back up…" },
	},
	{
		id: "a4",
		severity: "info",
		sentence: "An agent update is available.",
	},
];

describe("attention list", () => {
	test("tiers by severity, worded for screen readers, gated actions explain", async () => {
		const { container } = await dom.render(
			<At>
				<AttentionList items={ATTENTION_ITEMS} base="auto" />
			</At>,
		);
		expect(attrs(container, "[data-tier]", "data-tier")).toEqual([
			"now",
			"soon",
			"later",
			"info",
		]);
		byRole("region", "Broken now");
		const word = container.querySelector("[data-attention=a1] .sr-only");
		expect(word?.textContent).toBe("Critical");
		expect(container.textContent).toContain(
			"Can't be dismissed. Resolve to clear.",
		);
		const renew = byRole("button", "Renew…");
		expect(renew.getAttribute("aria-disabled")).toBe("true");
		expect(container.textContent).toContain("Unlock edge-berlin-01 first.");
		expect(queryByRole("button", "More for this item")).not.toBeNull();
		expect(allByRole("button", "More for this item")).toHaveLength(1);
		expect(
			container.querySelector("[data-attention=a1] [data-stamp]"),
		).toBeNull();
		expect(
			container.querySelector("[data-attention=a3] [data-stamp]"),
		).not.toBeNull();
		expect(container.querySelector("[data-tech]")).toBeNull();
		expect(container.textContent).not.toContain("Nothing needs you right now.");
	});

	test("narrow containers put the action under the sentence instead of squeezing it", async () => {
		const { container } = await dom.render(
			<At>
				<AttentionList items={ATTENTION_ITEMS} compact />
			</At>,
		);
		const list = container.querySelector("[data-tier=soon] ul");
		expect(classesOf(list)).toContain("@container/attn");
		const item = container.querySelector("[data-attention=a2]");
		expect(classesOf(item)).toContain(
			"@max-[560px]/attn:grid-cols-[18px_minmax(0,1fr)]",
		);
		const actions = item?.lastElementChild;
		expect(classesOf(actions)).toContain("@max-[560px]/attn:col-start-2");
		expect(classesOf(actions)).toContain("@max-[560px]/attn:justify-start");
		const gated = actions?.firstElementChild;
		expect(classesOf(gated)).toContain("@max-[560px]/attn:items-start");
		expect(classesOf(gated)).toContain("@max-[560px]/attn:text-left");
	});

	test("the snooze menu is a neutral menu (no coral highlight) and snoozes the item", async () => {
		let snoozed = 0;
		const notice: AttentionEntry = {
			...ATTENTION_ITEMS[2],
			onSnooze: () => {
				snoozed += 1;
			},
		};
		await dom.render(
			<At>
				<AttentionList items={[notice]} />
			</At>,
		);
		await click(byRole("button", "More for this item"));
		await settle();
		const menu = inPortal("menu");
		expect(menu.className).toContain("border-border-strong");
		expect(menu.className).toContain("backdrop-blur-none");
		const item = byRole("menuitem", "Snooze for 7 days", menu);
		expect(item.className).toContain("focus:bg-row-hover");
		expect(item.className).not.toContain("accent");
		await click(item);
		await settle();
		expect(snoozed).toBe(1);
	});

	test("technical keys only with the developer preference", async () => {
		const { container } = await dom.render(
			<At tech>
				<AttentionList items={ATTENTION_ITEMS.slice(0, 1)} />
			</At>,
		);
		expect(container.querySelector("[data-tech]")?.textContent).toBe(
			"offline_since",
		);
	});

	test("cap fills tiers in order; Show all counts everything but Info", async () => {
		let shown = 0;
		const { container } = await dom.render(
			<At>
				<AttentionList
					items={ATTENTION_ITEMS}
					cap={2}
					compact
					onShowAll={() => {
						shown += 1;
					}}
				/>
			</At>,
		);
		expect(container.querySelectorAll("[data-attention]")).toHaveLength(2);
		expect(container.textContent).not.toContain("Can't be dismissed");
		await click(byRole("button", "Show all 3"));
		expect(shown).toBe(1);
	});

	test("all clear and done results stay until dismissed", async () => {
		let dismissed = "";
		const { container } = await dom.render(
			<At>
				<AttentionList
					items={[ATTENTION_ITEMS[3]]}
					done={[
						{
							id: "d1",
							sentence: "Backed up to your account as version 1.",
							doneAt: NOW_S - 60,
							onDismiss: () => {
								dismissed = "d1";
							},
						},
					]}
				/>
			</At>,
		);
		expect(container.textContent).toContain("Nothing needs you right now.");
		expect(
			classesOf(
				byText("Nothing needs you right now.", container).parentElement,
			),
		).toContain("text-ui");
		byRole("region", "Done in this session");
		await click(byRole("button", "Dismiss"));
		expect(dismissed).toBe("d1");
	});

	test("device and service names in a sentence are set in mono, whole names only", async () => {
		const { container } = await dom.render(
			<At>
				<AttentionList
					items={[
						{
							id: "a1",
							severity: "critical",
							sentence:
								"support-bot on edge-berlin-01 keeps crashing; edge-berlin-011 is fine.",
							names: ["edge-berlin-01", "support-bot"],
						},
					]}
				/>
			</At>,
		);
		const item = container.querySelector("[data-attention=a1]");
		expect(texts(item as Element, "[data-obj]")).toEqual([
			"support-bot",
			"edge-berlin-01",
		]);
		expect(classesOf(item?.querySelector("[data-obj]"))).toContain("font-mono");
		expect(item?.textContent).toContain(
			"support-bot on edge-berlin-01 keeps crashing; edge-berlin-011 is fine.",
		);
		expect(monoNames("nothing named", ["edge-berlin-01"])).toBe(
			"nothing named",
		);
		const node = <b>already a node</b>;
		expect(monoNames(node, ["node"])).toBe(node);
	});
});

const ANNUNCIATOR_CELLS: Record<AnnunciatorWindow, AnnunciatorCell> = {
	critical: { count: 1, names: ["warehouse-pi"] },
	attention: {
		count: 4,
		names: ["edge-berlin-01", "studio-mac-mini", "cold-storage-nas", "lab-a"],
	},
	unknown: { count: 1, names: ["lab-gpu-02"], note: "locked" },
	healthy: { count: 0 },
	offline: { count: 1, names: ["warehouse-pi"], note: "also critical" },
	revoked: { count: 2, sub: "1 still billed to you" },
};

type HealthProps = Pick<
	ComponentProps<typeof Annunciator>,
	"pressed" | "onSelect"
>;

function Health(props: Readonly<HealthProps>) {
	return (
		<Annunciator
			cells={ANNUNCIATOR_CELLS}
			caption="Health counts each device once."
			{...props}
		/>
	);
}

const percent = (value: number) => `${value.toFixed(1)} %`;

const sparkRect = () => ({
	left: 0,
	top: 0,
	width: 240,
	height: 40,
	right: 240,
	bottom: 40,
});

const KEY_WINDOWS: AnnunciatorWindowSpec<
	"here" | "backed" | "bare" | "none"
>[] = [
	{ id: "here", tone: "neutral", icon: Laptop, label: "Keys here", count: 3 },
	{
		id: "backed",
		tone: "good",
		icon: Lock,
		label: "Backed up",
		count: 2,
		names: ["edge-berlin-01", "studio-mac-mini"],
	},
	{
		id: "bare",
		tone: "critical",
		icon: Fingerprint,
		label: "Not backed up",
		count: 1,
		title: "cold-storage-nas has no backup",
	},
	{
		id: "none",
		tone: "unknown",
		icon: Users,
		label: "No keys here",
		count: 0,
	},
];

describe("summary pieces", () => {
	test("annunciator: six windows, lit by count, press toggles the filter", async () => {
		const picked: (string | null)[] = [];
		const onSelect = (next: string | null) => {
			picked.push(next);
		};
		const { container, rerender } = await dom.render(
			<Health pressed={null} onSelect={onSelect} />,
		);
		const windows = Array.from(container.querySelectorAll("[data-window]"));
		expect(attrs(container, "[data-window]", "data-window")).toEqual([
			"critical",
			"attention",
			"unknown",
			"healthy",
			"offline",
			"revoked",
		]);
		expect(attrs(container, "[data-window]", "data-lit")).toEqual([
			"true",
			"true",
			"true",
			"false",
			"true",
			"true",
		]);
		expect(windows[0].className).toContain("bg-critical-bg");
		expect(windows[3].textContent).toContain("none right now");
		expect(windows[1].textContent).toContain(
			"edge-berlin-01, studio-mac-mini, cold-storage-nas +1 more",
		);
		expect(windows[2].textContent).toContain("lab-gpu-02 · locked");
		expect(windows[5].textContent).toContain("1 still billed to you");
		byRole("group", "Fleet health. Select a window to filter devices.");
		await click(windows[0]);
		await rerender(<Health pressed="critical" onSelect={onSelect} />);
		const pressed = container.querySelector("[data-window=critical]");
		expect(pressed?.getAttribute("aria-pressed")).toBe("true");
		if (pressed) await click(pressed);
		expect(picked).toEqual(["critical", null]);
	});

	test("annunciator windows are data-driven: another summary brings its own windows", async () => {
		const picked: (string | null)[] = [];
		const { container } = await dom.render(
			<AnnunciatorWindows
				label="Keys summary. Select a window to filter the device list."
				pressed="backed"
				onSelect={(next) => picked.push(next)}
				windows={KEY_WINDOWS}
			/>,
		);
		byRole("group", "Keys summary. Select a window to filter the device list.");
		expect(attrs(container, "[data-window]", "data-window")).toEqual([
			"here",
			"backed",
			"bare",
			"none",
		]);
		const [here, backed, bare, none] = Array.from(
			container.querySelectorAll("[data-window]"),
		);
		expect(classesOf(here.firstElementChild)).toContain("text-foreground");
		expect(classesOf(backed.firstElementChild)).toContain("text-good");
		expect(backed.getAttribute("aria-pressed")).toBe("true");
		expect(backed.getAttribute("title")).toBe(
			"edge-berlin-01, studio-mac-mini",
		);
		expect(bare.className).toContain("bg-critical-bg");
		expect(bare.getAttribute("title")).toBe("cold-storage-nas has no backup");
		expect(none.textContent).toContain("none right now");
		await click(backed);
		await click(bare);
		expect(picked).toEqual([null, "bare"]);
	});

	test("annunciator: a count without names never reads as none", () => {
		expect(annunciatorSub(t, { count: 12 })).toBe("");
		expect(annunciatorSub(t, { count: 2, note: "locked" })).toBe("locked");
		expect(annunciatorSub(t, { count: 0, names: ["old-kiosk"] })).toBe(
			"none right now",
		);
	});

	test("person initials keep whole characters", () => {
		expect(initialsOf("Mira Novak")).toBe("MN");
		expect(initialsOf("😀 Bob")).toBe("😀B");
		expect(initialsOf("𝒜da")).toBe("𝒜D");
		expect(initialsOf("  ")).toBe("?");
	});

	test("metric sparkline is inline SVG with a spoken range and a hover tip", async () => {
		const { container } = await dom.render(
			<At>
				<MetricGrid>
					<Metric
						label="CPU"
						value="23.4"
						unit="%"
						note="8 logical CPUs · 100 % = all of them"
						axis={["13:30", "max 100 %", "now"]}
						spark={{
							series: [18, 22, 31, 26.1, 23.4],
							label: "CPU",
							min: 0,
							max: 100,
							format: percent,
							startAt: NOW_S - 4 * 60,
							span: "the last 4 minutes",
						}}
					/>
				</MetricGrid>
			</At>,
		);
		const spark = byRole(
			"img",
			"CPU over the last 4 minutes, 18.0 % to 31.0 %, now 23.4 %",
		);
		expect(spark.querySelector("svg path")).not.toBeNull();
		expect(container.querySelector("[class*=recharts]")).toBeNull();
		expect(container.textContent).toContain("max 100 %");
		Object.defineProperty(spark, "getBoundingClientRect", { value: sparkRect });
		const view = spark.ownerDocument
			.defaultView as unknown as typeof globalThis;
		await fire(
			spark,
			new view.PointerEvent("pointermove", { bubbles: true, clientX: 240 }),
		);
		const tip = container.querySelector("[data-spark-tip]");
		expect(tip?.textContent).toEndWith(" · 23.4 %");
	});

	test("sparkline: a gap keeps the later points at their own place and time", async () => {
		const { container } = await dom.render(
			<At>
				<Metric
					label="CPU"
					value="30.0"
					spark={{
						series: [10, Number.NaN, Number.NaN, 30],
						label: "CPU",
						min: 0,
						max: 100,
						format: percent,
						startAt: NOW_S - 180,
						stepSec: 60,
					}}
				/>
			</At>,
		);
		const spark = container.querySelector("[data-sparkline]") as HTMLElement;
		const line = spark.querySelectorAll("svg path")[1]?.getAttribute("d") ?? "";
		expect(line).toStartWith("M0.0,");
		expect(line).toContain("L240.0,");
		Object.defineProperty(spark, "getBoundingClientRect", { value: sparkRect });
		const view = spark.ownerDocument
			.defaultView as unknown as typeof globalThis;
		await fire(
			spark,
			new view.PointerEvent("pointermove", { bubbles: true, clientX: 240 }),
		);
		const readAt = formatTimeOfDay(NOW_S * 1000, {
			locale: "en",
			seconds: false,
		});
		expect(container.querySelector("[data-spark-tip]")?.textContent).toBe(
			`${readAt} · 30.0 %`,
		);
	});

	test("meters state their numbers; progress bars are labelled", async () => {
		const { container } = await dom.render(
			<>
				<Meter
					label="2 live, 1 snapshot"
					segments={[
						{ value: 40, tone: "good" },
						{ value: 20, tone: "unknown" },
					]}
				/>
				<SpendMeter used={7.41} reserved={0.12} limit={25} />
				<ProgressBar value={62} label="Safe update progress" />
				<ProgressBar label="Upload progress" />
			</>,
		);
		byRole("img", "2 live, 1 snapshot");
		expect(container.textContent).toContain(
			"€7.41 used · €0.12 reserved · €25.00 limit",
		);
		expect(
			byRole("progressbar", "Safe update progress").getAttribute(
				"aria-valuenow",
			),
		).toBe("62");
		expect(
			byRole("progressbar", "Upload progress").hasAttribute(
				"data-indeterminate",
			),
		).toBe(true);
	});

	test("a spending meter leaves out a zero reserve; its caption runs past a short bar", async () => {
		const { container } = await dom.render(
			<SpendMeter
				used={7.41}
				reserved={0}
				limit={25}
				tail="paid by you · ends 5 Oct"
				className="w-full"
				meterClassName="w-70"
			/>,
		);
		const text = "€7.41 used · €25.00 limit";
		expect(container.textContent).toBe(`${text} · paid by you · ends 5 Oct`);
		const bar = byRole("img", text);
		expect(classesOf(bar.parentElement)).toContain("w-70");
		const caption = byText(`${text} · paid by you · ends 5 Oct`, container);
		expect(bar.parentElement?.contains(caption)).toBe(false);
		expect(classesOf(caption.parentElement)).toContain("w-full");
	});

	test("a spending meter survives a currency code the runtime rejects", async () => {
		const { container } = await dom.render(
			<SpendMeter used={7.41} reserved={0.12} limit={25} currency="EURO!" />,
		);
		expect(container.textContent).toBe(
			"7.41 used · 0.12 reserved · 25.00 limit",
		);
	});

	test("coverage line, headline and person chip", async () => {
		const { container } = await dom.render(
			<At>
				<CoverageLine
					readable={3}
					total={5}
					live={2}
					snapshot={1}
					unknown={1}
					never={1}
					actions={<DvButton size="sm">Unlock 1…</DvButton>}
				/>
				<Headline
					lead="warehouse-pi needs you now."
					rest="8 things need a look soon."
				/>
				<PersonChip name="Mira Novak" email="mira@example.com" />
				<PersonChip name="Felix Schultz" you />
			</At>,
		);
		const coverage = container.querySelector("[data-coverage]");
		expect(coverage?.textContent).toContain(
			"Status from 3 of 5 devices you can see",
		);
		expect(coverage?.querySelector("b")?.textContent).toBe("3 of 5");
		expect(coverage?.textContent).toContain("· 1 unknown");
		expect(coverage?.textContent).toContain("· 1 hasn't checked in");
		byRole("img", "2 live, 1 from encrypted snapshots, 2 unknown");
		expect(container.querySelector("[data-headline]")?.textContent).toBe(
			"warehouse-pi needs you now.8 things need a look soon.",
		);
		const people = Array.from(container.querySelectorAll("[data-person]"));
		expect(people[0].getAttribute("title")).toBe(
			"Mira Novak · mira@example.com",
		);
		expect(people[0].textContent).toBe("MNMira Novak");
		expect(people[1].textContent).toBe("FSYou");
	});

	test("headline: the names it is given are set in mono in both sentences", async () => {
		const { container } = await dom.render(
			<Headline
				lead="invoice-extractor on edge-berlin-01 is switching to settings v12."
				rest="warehouse-pi has been offline since 11:00."
				names={["edge-berlin-01", "invoice-extractor", "warehouse-pi"]}
			/>,
		);
		const headline = container.querySelector("[data-headline]");
		expect(texts(headline as Element, "[data-obj]")).toEqual([
			"invoice-extractor",
			"edge-berlin-01",
			"warehouse-pi",
		]);
		expect(classesOf(headline?.querySelector("[data-obj]"))).toContain(
			"text-[0.9em]",
		);
		expect(headline?.textContent).toBe(
			"invoice-extractor on edge-berlin-01 is switching to settings v12.warehouse-pi has been offline since 11:00.",
		);
	});

	test("days and distances: one formatter for the area", async () => {
		const time = {
			locale: "en-GB",
			now: NOW_S * 1000,
			nowS: NOW_S,
			ago: () => "in 30 min.",
		};
		expect(dayText(time, NOW_S + 5 * DAY)).toBe(
			new Intl.DateTimeFormat("en-GB", {
				day: "numeric",
				month: "short",
			}).format((NOW_S + 5 * DAY) * 1000),
		);
		expect(dayText(time, NOW_S + 400 * DAY)).toMatch(/\d{4}$/);
		// A count, where `time.ago` would say "tomorrow" and "next mo.".
		const count = new Intl.RelativeTimeFormat("en-GB", {
			numeric: "always",
			style: "narrow",
		});
		expect(untilText(time, NOW_S + 24 * 3600)).toBe(count.format(24, "hour"));
		expect(untilText(time, NOW_S + 24 * 3600)).toContain("24");
		expect(untilText(time, NOW_S + 31 * DAY)).toBe(count.format(31, "day"));
		expect(untilText(time, NOW_S + 31 * DAY)).toContain("31");
		expect(untilText(time, NOW_S - 3 * DAY)).toBe(count.format(-3, "day"));
		expect(untilText(time, NOW_S + 1800)).toBe("in 30 min.");
		const { container } = await dom.render(
			<At>
				<DayOf at={NOW_S + 5 * DAY} />
			</At>,
		);
		const day = container.querySelector("time");
		expect(day?.getAttribute("datetime")).toBe(
			new Date((NOW_S + 5 * DAY) * 1000).toISOString(),
		);
		expect(day?.getAttribute("title")).toBeTruthy();
	});
});

const CHECKIN_TICKS = [
	...Array.from({ length: 4 }, () => "pre" as const),
	...Array.from({ length: 80 }, () => "ok" as const),
	...Array.from({ length: 12 }, () => "miss" as const),
];

const TRUST_LINKS: ChainLink[] = [
	{
		id: "keys",
		icon: Laptop,
		state: "good",
		title: "Keys on this computer",
		text: "Owner keys · backed up (v3)",
		join: "you trusted this device's keys on 14 Mar 2026",
	},
	{
		id: "id",
		icon: Fingerprint,
		state: "critical",
		title: "Device identity",
		text: "Doesn't match the hub · management is blocked",
		join: "enforces access rules v5",
	},
	{
		id: "rules",
		icon: Users,
		state: "locked",
		title: "Access rules v5",
		text: "Locked",
	},
];

const TIMELINE_ROWS: TimelineRow[] = [
	{
		id: "e1",
		at: NOW_S - 60,
		kind: "instance",
		tone: "good",
		text: "Instance #0 is ready.",
		meta: "process 48211",
	},
	{ id: "e2", at: NOW_S - 120, kind: "command", text: "You restarted it." },
	{ id: "g1", gap: true, text: "Older entries were deleted to save space." },
	{
		id: "e3",
		at: NOW_S - DAY,
		kind: "access",
		text: "Access rules v5 applied.",
	},
];

describe("time rails and chains", () => {
	test("check-in lane: 96 fixed-width ticks with a sentence", async () => {
		const { container } = await dom.render(
			<At>
				<CheckinLane ticks={CHECKIN_TICKS} />
			</At>,
		);
		const lane = container.querySelector("[data-lane]");
		const label = lane?.getAttribute("aria-label") ?? "";
		expect(lane?.getAttribute("role")).toBe("img");
		expect(label).toStartWith(
			"Checked in during 80 of the last 92 quarter-hours. Missed every one since ",
		);
		expect(label).toEndWith(".");
		expect(lane?.querySelector("svg")?.getAttribute("width")).toBe("112");
		expect(container.querySelectorAll("[data-tick=ok]")).toHaveLength(80);
		expect(container.querySelectorAll("[data-tick=miss]")).toHaveLength(12);
		expect(container.querySelectorAll("[data-tick=pre]")).toHaveLength(4);
	});

	test("expiry rail: state by days left, arrow beyond +90 d, a shared scale", async () => {
		const { container } = await dom.render(
			<At>
				<ExpiryRail notAfter={NOW_S - 3 * DAY} />
				<ExpiryRail notAfter={NOW_S + 5 * DAY} />
				<ExpiryRail notAfter={NOW_S + 40 * DAY} />
				<ExpiryRail notAfter={NOW_S + 200 * DAY} />
				<ExpiryRailScale />
			</At>,
		);
		expect(attrs(container, "[data-xrail]", "data-xrail")).toEqual([
			"expired",
			"expiring",
			"valid",
			"valid",
		]);
		const [expired, expiring] = attrs(container, "[data-xrail]", "aria-label");
		expect(expired).toStartWith("Expired ");
		expect(expired).toEndWith(", 3 days ago");
		expect(expiring).toStartWith("Expires ");
		expect(expiring).toEndWith(", in 5 days");
		const rails = container.querySelectorAll("[data-xrail]");
		expect(rails[3].querySelector("[data-mark=beyond]")).not.toBeNull();
		expect(container.textContent).toContain("today");
	});

	test("expiry rail: an expiry that isn't a date draws nothing instead of throwing", async () => {
		const { container } = await dom.render(
			<At>
				<ExpiryRail notAfter={Number.NaN} />
				<ExpiryRail notAfter={1e18} />
			</At>,
		);
		expect(container.querySelector("[data-xrail]")).toBeNull();
	});

	test("trust chain: joins between links, dropped when compact", async () => {
		const { container, rerender } = await dom.render(
			<TrustChain links={TRUST_LINKS} />,
		);
		expect(container.querySelectorAll("[data-join]")).toHaveLength(2);
		const critical = container.querySelector("[data-state=critical] span");
		expect(critical?.className).toContain("rounded-md");
		expect(container.textContent).toContain("Device identity (Broken)");
		await rerender(<TrustChain links={TRUST_LINKS} compact />);
		expect(container.querySelectorAll("[data-join]")).toHaveLength(0);
	});

	test("timeline: day groups, filters, gaps never filtered out", async () => {
		let older = 0;
		const { container } = await dom.render(
			<At>
				<Timeline
					rows={TIMELINE_ROWS}
					onLoadOlder={() => {
						older += 1;
					}}
				/>
			</At>,
		);
		expect(texts(container, "h4")).toEqual(["Today", "Yesterday"]);
		expect(container.querySelectorAll("[data-kind]")).toHaveLength(3);
		expect(classesOf(container.querySelector("[data-kind] p"))).toContain(
			"text-ui",
		);
		await click(byRole("button", "Commands"));
		expect(container.querySelectorAll("[data-kind]")).toHaveLength(1);
		expect(container.querySelectorAll("[data-gap]")).toHaveLength(1);
		await click(byRole("button", "Updates"));
		expect(container.querySelectorAll("[data-kind]")).toHaveLength(0);
		expect(container.textContent).toContain("Older entries were deleted");
		await clickByText("Load older");
		expect(older).toBe(1);
	});

	test("timeline: a broken device timestamp doesn't take the screen down", async () => {
		const { container } = await dom.render(
			<At>
				<Timeline
					rows={[
						{ id: "nan", at: Number.NaN, kind: "device", text: "No time." },
						{ id: "far", at: 1e18, kind: "device", text: "Microseconds." },
						TIMELINE_ROWS[0],
					]}
				/>
			</At>,
		);
		expect(container.querySelectorAll("[data-kind]")).toHaveLength(3);
		expect(attrs(container, "time", "datetime")).toEqual([
			null,
			null,
			new Date((NOW_S - 60) * 1000).toISOString(),
		]);
		expect(texts(container, "h4")).toEqual(["Unknown time", "Today"]);
	});

	test("timeline: empty says so; an empty filter says it is the filter", async () => {
		const { container, rerender } = await dom.render(
			<At>
				<Timeline rows={[]} />
			</At>,
		);
		expect(container.textContent).toContain("No entries yet.");
		await rerender(
			<At>
				<Timeline rows={[]} filter="updates" onFilterChange={() => {}} />
			</At>,
		);
		expect(container.textContent).toContain("No entries match this filter.");
	});
});

const logLine = (index: number, count: number) => {
	const line: LogRecord = {
		kind: "line",
		id: `l${index}`,
		at: NOW_S - count + index,
		stream: index % 5 === 0 ? "stderr" : "stdout",
		message: `line ${index}`,
	};
	return line;
};

const logLines = (count: number) =>
	Array.from({ length: count }, (_, index) => logLine(index, count));

const SHORT_LOG: LogRecord[] = [
	{ kind: "gap", id: "g0", reason: "evicted", before: 88_000 },
	...logLines(10),
	{ kind: "gap", id: "g1", reason: "dropped", count: 37 },
	{
		kind: "line",
		id: "cut",
		at: NOW_S,
		stream: "stdout",
		message: "big",
		truncated: true,
	},
];

const LONG_LOG: LogRecord[] = [
	{ kind: "gap", id: "g", reason: "dropped", count: 1 },
	...logLines(800),
];

const LAYOUT_KEYS = ["offsetHeight", "offsetWidth"] as const;

/** happy-dom has no layout; the virtualizer needs a viewport to window into. Returns the undo. */
function fakeViewport(height: number, width: number) {
	const proto = window.HTMLElement.prototype;
	const saved = LAYOUT_KEYS.map((key) => {
		return [key, Object.getOwnPropertyDescriptor(proto, key)] as const;
	});
	Object.defineProperty(proto, "offsetHeight", {
		configurable: true,
		value: height,
	});
	Object.defineProperty(proto, "offsetWidth", {
		configurable: true,
		value: width,
	});
	return () => {
		for (const [key, descriptor] of saved) {
			if (descriptor) Object.defineProperty(proto, key, descriptor);
			else Reflect.deleteProperty(proto, key);
		}
	};
}

describe("logs and commands", () => {
	test("short buffers render whole with gap markers; Errors only keeps stderr and gaps", async () => {
		const { container } = await dom.render(
			<At>
				<LogViewer records={SHORT_LOG} label="invoice-extractor output" />
			</At>,
		);
		const log = byRole("log", "invoice-extractor output");
		expect(log.hasAttribute("data-windowed")).toBe(false);
		expect(log.querySelectorAll("[data-stream]")).toHaveLength(11);
		expect(log.textContent).toContain(
			"Older lines were deleted on the device (before #88,000).",
		);
		expect(log.textContent).toContain(
			"37 lines dropped here: more than 100 lines per second.",
		);
		expect(log.textContent).toContain("[line cut at 2 048 bytes]");
		expect(log.querySelector("[data-stream=stderr]")?.textContent).toContain(
			"Errors",
		);
		await click(byRole("button", "Errors only"));
		expect(log.querySelectorAll("[data-stream=stdout]")).toHaveLength(0);
		expect(log.querySelectorAll("[data-stream=stderr]")).toHaveLength(2);
		expect(log.querySelectorAll("[data-gap]")).toHaveLength(2);
		await click(byRole("button", "Pause"));
		expect(
			container.querySelector("[data-follow]")?.getAttribute("data-follow"),
		).toBe("false");
		byRole("button", "Follow");
	});

	test("follow keeps up when a full buffer swaps lines; Errors only names an empty result", async () => {
		const window3 = (from: number) =>
			[0, 1, 2].map((offset) => {
				const line: LogRecord = {
					kind: "line",
					id: `w${from + offset}`,
					at: NOW_S + from + offset,
					stream: "stdout",
					message: `line ${from + offset}`,
				};
				return line;
			});
		const { rerender } = await dom.render(
			<At>
				<LogViewer records={window3(0)} label="Agent logs" />
			</At>,
		);
		const log = byRole("log", "Agent logs");
		Object.defineProperty(log, "scrollHeight", {
			configurable: true,
			value: 640,
		});
		await rerender(
			<At>
				<LogViewer records={window3(1)} label="Agent logs" />
			</At>,
		);
		expect(log.scrollTop).toBe(640);
		await click(byRole("button", "Errors only"));
		expect(log.textContent).toBe("No error lines.");
	});

	describe("long buffers", () => {
		let restore = () => {};
		beforeAll(() => {
			restore = fakeViewport(320, 600);
		});
		afterAll(() => {
			restore();
		});

		test("are windowed", async () => {
			await dom.render(
				<At>
					<LogViewer
						records={LONG_LOG}
						label="Agent logs"
						follow={false}
						onFollowChange={() => {}}
					/>
				</At>,
			);
			await settle();
			const log = byRole("log", "Agent logs");
			expect(log.hasAttribute("data-windowed")).toBe(true);
			const rendered = log.querySelectorAll("[data-stream]").length;
			expect(rendered).toBeGreaterThan(0);
			expect(rendered).toBeLessThan(200);
			expect(log.textContent).toContain("1 line dropped here");
		});
	});

	test("command block copies the command", async () => {
		await dom.render(
			<CommandBlock
				command="flow-like-standalone status"
				note="Shows the connection state."
			/>,
		);
		await click(byRole("button", "Copy command: flow-like-standalone status"));
		expect(dom.clipboard).toEqual(["flow-like-standalone status"]);
		byRole("button", "Copied");
	});

	test("checklist speaks each state", async () => {
		await dom.render(
			<Checklist
				label="Pre-flight"
				items={[
					{ id: "a", state: "pass", label: "Hub reachable", source: "hub" },
					{ id: "b", state: "fail", label: "Keys here", fix: "Restore keys…" },
					{ id: "c", state: "active", label: "Connecting" },
				]}
			/>,
		);
		byRole("img", "Passed");
		byRole("img", "Failed");
		byRole("img", "Checking");
	});
});

const SUMMARY_ITEMS: WizardSummaryItem[] = [
	{
		id: "1",
		label: "Check",
		state: "done",
		value: "Ready",
		onSelect: () => {},
	},
	{ id: "2", label: "Platform", state: "err", errors: 2 },
	{ id: "3", label: "Device password", state: "current" },
];

function PasswordStep({ onNext }: Readonly<Pick<WizardFootProps, "onNext">>) {
	return (
		<wizard.WizardLayout
			stepper={
				<wizard.WizardStepper
					steps={["Check", "Platform", "Device password", "Create"]}
					current={2}
				/>
			}
			side={
				<wizard.WizardSummary
					items={SUMMARY_ITEMS}
					foot="Nothing changes on any device until you deploy."
				/>
			}
			foot={
				<wizard.WizardFoot
					step={3}
					total={4}
					stepLabel="Device password"
					nextStepLabel="Create"
					onBack={() => {}}
					onNext={onNext}
				/>
			}
		>
			<wizard.WizardStepHeader
				title="Device password"
				lede="It protects the setup file."
				step={3}
				total={4}
			/>
		</wizard.WizardLayout>
	);
}

const TRAY_STATES = [
	"active",
	"paused",
	"waiting",
	"done",
	"failed",
	"unknown",
] as const;

describe("wizard and tray", () => {
	test("stepper marks the current step; the foot has exactly one primary", async () => {
		let next = 0;
		const { container } = await dom.render(
			<PasswordStep
				onNext={() => {
					next += 1;
				}}
			/>,
		);
		const current = container.querySelector("[aria-current=step]");
		expect(current?.textContent).toContain("Device password");
		expect(current?.className).toContain("data-[s=current]:min-w-fit");
		const label = current?.querySelector("[title]");
		expect(label?.getAttribute("title")).toBe("Device password");
		expect(container.textContent).toContain("Step 3 of 4 · Device password");
		expect(container.querySelectorAll("[data-dv-primary]")).toHaveLength(1);
		expect(container.textContent).toContain("Platform · 2 to fix");
		byRole("heading", "Step 3 of 4: Device password");
		await click(byRole("button", "Continue"));
		expect(next).toBe(1);
		byRole("button", "CheckReady");
	});

	test("a gated Continue stays visible with its reason", async () => {
		await dom.render(
			<wizard.WizardFoot
				step={8}
				total={8}
				stepLabel="Rollout"
				onNext={() => {}}
				nextLabel="Update nightly-sync"
				nextGate={{ kind: "live", reason: "Needs a live connection." }}
			/>,
		);
		const button = byRole("button", "Update nightly-sync");
		expect(button.getAttribute("aria-disabled")).toBe("true");
		byText("Needs a live connection.");
	});

	test("foot: a reason takes the note area at every width; a locked Back says why; a result sits above", async () => {
		let moved = 0;
		const foot = (extra: Partial<WizardFootProps>) => (
			<wizard.WizardFoot
				step={3}
				total={5}
				stepLabel="Create"
				nextStepLabel="Start"
				onBack={() => {
					moved += 1;
				}}
				onNext={() => {
					moved += 1;
				}}
				{...extra}
			/>
		);
		const { container, rerender } = await dom.render(
			foot({
				reason: { kind: "busy", reason: "Enter the device password first." },
				backGate: {
					kind: "locked",
					reason: "Earlier steps are locked: the package is built.",
				},
				result: <span data-result="">The setup couldn't be cancelled.</span>,
			}),
		);
		const root = container.querySelector("[data-wizard-foot]") as HTMLElement;
		expect(root.firstElementChild?.className).toBe("basis-full");
		expect(root.firstElementChild?.textContent).toBe(
			"The setup couldn't be cancelled.",
		);
		const reason = byText(
			"Enter the device password first.",
			container,
		).closest("[data-gate-inline]") as HTMLElement;
		expect(reason.getAttribute("data-gate-inline")).toBe("busy");
		// Not the hidden-on-phones <output>: the reason gets its own row above the buttons.
		expect(container.querySelector("output")).toBeNull();
		expect(classesOf(reason.parentElement)).toContain(
			"@max-[720px]/wfoot:basis-full",
		);
		const next = byRole("button", "Continue");
		const back = byRole("button", "Back");
		expect(next.getAttribute("aria-disabled")).toBe("true");
		expect(next.getAttribute("aria-describedby")).toBe(reason.id);
		expect(back.getAttribute("aria-disabled")).toBe("true");
		expect(
			document.getElementById(back.getAttribute("aria-describedby") ?? "")
				?.textContent,
		).toBe("Earlier steps are locked: the package is built.");
		await click(next);
		await click(back);
		expect(moved).toBe(0);

		await rerender(
			foot({
				backGate: {
					kind: "locked",
					reason: "Earlier steps are locked: the package is built.",
				},
			}),
		);
		expect(
			byText("Earlier steps are locked: the package is built.", container)
				.closest("[data-gate-inline]")
				?.getAttribute("data-gate-inline"),
		).toBe("locked");
		await click(byRole("button", "Continue"));
		expect(moved).toBe(1);

		await rerender(foot({}));
		expect(container.querySelector("output")?.textContent).toBe(
			"Step 3 of 5 · Create · next: Start",
		);
	});

	test("stepper: prefilled steps are done, reachable ones are buttons, the phone line takes the title", async () => {
		const went: number[] = [];
		const { container } = await dom.render(
			<wizard.WizardStepper
				steps={["What", "How it runs", "Where", "Settings"]}
				titles={["What to run", "How it runs", "Where it runs", "Settings"]}
				current={1}
				done={[true, false, true, false]}
				reachable={[true, true, true, false]}
				onSelect={(index) => went.push(index)}
				fit
			/>,
		);
		expect(attrs(container, "ol > li", "data-s")).toEqual([
			"done",
			"current",
			"done",
			"todo",
		]);
		expect(texts(container, "ol button")).toEqual([
			"What (done)",
			"Where (done)",
		]);
		await click(byRole("button", "Go to step 3: Where it runs"));
		expect(went).toEqual([2]);
		expect(container.textContent).toContain("Step 2 of 4 · How it runs");
		const [first] = Array.from(container.querySelectorAll("ol > li"));
		expect(first.className).toContain("flex-auto");
		expect(first.className).not.toContain("flex-1");
		// Labels wrap instead of ending in "…" in a narrow window or area.
		expect(classesOf(first.querySelector("[title]"))).toContain(
			"max-[1200px]:whitespace-normal",
		);
		expect(classesOf(first.querySelector("[title]"))).toContain(
			"@max-[1100px]/devices:whitespace-normal",
		);
	});

	test.each(["err", "busy"] as const)(
		"stepper: %s overrides completion while the current step and navigation remain accessible",
		async (state) => {
			const went: number[] = [];
			const { container } = await dom.render(
				<wizard.WizardStepper
					steps={["What", "How it runs", "Where"]}
					current={1}
					done={[true, true, true]}
					states={[state, state]}
					reachable={[true, true, true]}
					onSelect={(index) => went.push(index)}
				/>,
			);
			expect(attrs(container, "ol > li", "data-s")).toEqual([
				state,
				state,
				"done",
			]);
			const current = container.querySelector("[aria-current=step]");
			expect(current?.textContent).toContain("How it runs");
			expect(current?.querySelector("button")).toBeNull();
			expect(container.querySelectorAll("[aria-current=step]")).toHaveLength(1);
			const label = state === "err" ? " (Failed)" : " (Checking)";
			expect(current?.querySelector(".sr-only")?.textContent).toBe(label);
			const button = byRole("button", "Go to step 1: What");
			expect(
				document.getElementById(button.getAttribute("aria-describedby") ?? "")
					?.textContent,
			).toBe(label);
			if (state === "busy") {
				expect(
					current?.querySelector("svg")?.classList.contains("animate-spin"),
				).toBe(true);
				expect(classesOf(current?.querySelector("svg"))).toContain(
					"motion-reduce:animate-none",
				);
			} else {
				expect(classesOf(current?.querySelector("[title]"))).toContain(
					"text-critical",
				);
			}
			await click(button);
			expect(went).toEqual([0]);
		},
	);

	test("summary: work in progress and failures have spoken statuses", async () => {
		const { container } = await dom.render(
			<wizard.WizardSummary
				items={[
					{ id: "prepare", label: "Prepare", state: "busy", value: "Waiting" },
					{ id: "validate", label: "Validate", state: "err", value: "Retry" },
				]}
			/>,
		);
		expect(texts(container, ".sr-only")).toEqual(["(Checking)", "(Failed)"]);
		const spinner = container.querySelector("li[data-s=busy] svg");
		expect(classesOf(spinner)).toContain("animate-spin");
		expect(classesOf(spinner)).toContain("motion-reduce:animate-none");
		expect(classesOf(container.querySelector("li[data-s=err] svg"))).toContain(
			"text-critical",
		);
	});

	test("title row stacks by its own wrapper; the layout takes a row above both columns", async () => {
		const { container } = await dom.render(
			<wizard.WizardLayout
				top={<span data-top="">stepper</span>}
				side={<span>side</span>}
			>
				<wizard.WizardTitleRow
					exitLabel="Exit deploy"
					title="Deploy Invoice AI"
					className="flex-[1_1_420px]"
				/>
			</wizard.WizardLayout>,
		);
		expect(
			classesOf(container.querySelector("[data-top]")?.parentElement),
		).toContain("col-span-full");
		const row = byRole("button", "Exit deploy").parentElement as HTMLElement;
		expect(row.className).toContain("@max-[720px]/wtitle:flex-col");
		expect(row.className).not.toContain("@container/wtitle");
		// A container query never matches the container itself: the wrapper is the container.
		expect(classesOf(row.parentElement)).toContain("@container/wtitle");
		expect(classesOf(row.parentElement)).toContain("flex-[1_1_420px]");
	});

	test("tray items: every state has a chip; unknown asks to check", async () => {
		let dismissed = 0;
		const onDismiss = () => {
			dismissed += 1;
		};
		const { container } = await dom.render(
			<At>
				{TRAY_STATES.map((state) => {
					return (
						<TrayItem
							key={state}
							kind="safe_update"
							state={state}
							title={`Safe update ${state}`}
							sub="edge-berlin-01 › invoice-extractor"
							progress={state === "active" ? 62 : undefined}
							startedAt={NOW_S - 120}
							by="you"
							onDismiss={onDismiss}
						/>
					);
				})}
			</At>,
		);
		expect(attrs(container, "[data-op]", "data-state")).toEqual([
			...TRAY_STATES,
		]);
		const ops = Array.from(container.querySelectorAll("[data-op]"));
		for (const op of ops)
			expect(op.querySelector("[data-slot=badge]")).not.toBeNull();
		expect(ops[5].textContent).toContain("No reply received");
		expect(ops[5].textContent).toContain("It may have run.");
		expect(
			byRole("progressbar", "Safe update active progress").getAttribute(
				"aria-valuenow",
			),
		).toBe("62");
		expect(allByRole("button", "Dismiss")).toHaveLength(2);
		await click(allByRole("button", "Dismiss")[0]);
		expect(dismissed).toBe(1);
	});

	test("only a running operation animates its bar", async () => {
		const { container } = await dom.render(
			<At>
				{TRAY_STATES.map((state) => {
					return (
						<TrayItem
							key={state}
							kind="command"
							state={state}
							title={`Stop ${state}`}
						/>
					);
				})}
			</At>,
		);
		const moving = Array.from(
			container.querySelectorAll("[data-indeterminate]"),
			(bar) => bar.closest("[data-op]")?.getAttribute("data-state"),
		);
		expect(moving).toEqual(["active"]);
		expect(
			byRole("progressbar", "Stop failed progress").getAttribute(
				"aria-valuenow",
			),
		).toBe("0");
	});
});

function matrixCells(unlock: ReactNode) {
	const cells: MatrixCellProps[] = [
		{
			state: "served",
			serviceId: "invoice-extractor",
			href: "#svc",
			desired: "running",
			observed: "running",
			conv: "converged",
			actual: "Running",
			pin: "1.4.0 · flow 2.1.0",
			target: "1.5.0",
			port: ":8081",
			tls: true,
		},
		{
			state: "staged",
			serviceId: "support-bot",
			desired: "running",
			observed: "running",
			conv: "converged",
			actual: "Running",
			pin: "1.5.0 · flow 2.2.0",
			stagedVersion: "1.5.0",
			onActivate: () => {},
		},
		{
			state: "not_served",
			onDeploy: () => {},
			gate: { kind: "live", reason: "Offline since 11:00" },
		},
		{
			state: "cant_here",
			reason: "edge-berlin-01 requires sandboxed services.",
		},
		{ state: "unknown", why: "locked", action: unlock },
		{ state: "unknown", why: "snapshot" },
		{ state: "no_access" },
	];
	return cells;
}

const cellKey = (cell: MatrixCellProps) =>
	"why" in cell ? `${cell.state}:${cell.why}` : cell.state;

const MATRIX_TEXTS = [
	"→ 1.5.0",
	"newest",
	"1.5.0 staged",
	"Not served",
	"Offline since 11:00",
	"Can't run here",
	"Unknown until unlocked",
	"Unknown: the status snapshot has no event list",
	"Shared for another app",
];

const UPDATING = {
	desired: "running",
	observed: "running",
	conv: "update_in_progress",
} as const;

const SERVICE_LABELS = [
	"Service",
	"Requested → actual",
	"Instances",
	"Versions",
	"Update",
	"Actions",
];

describe("service rows and app pieces", () => {
	test("service row: every cell is labelled for cards; row click skips controls", async () => {
		let opened = 0;
		let stopped = 0;
		const onStop = () => {
			stopped += 1;
		};
		const { container } = await dom.render(
			<At>
				<DvTable
					label="Services"
					cols={SERVICE_ROW_COLS}
					head={<ServiceRowHead />}
				>
					<ServiceRow
						serviceId="invoice-extractor"
						app={{ name: "Invoice AI", href: "#app" }}
						state={UPDATING}
						pins="titled"
						instances={{ ready: 0, requested: 1, max: 1 }}
						version="Settings v12"
						versionSub="running v11 · applying"
						actions={
							<DvButton size="sm" onClick={onStop}>
								Stop…
							</DvButton>
						}
						onOpen={() => {
							opened += 1;
						}}
					/>
				</DvTable>
			</At>,
		);
		expect(texts(container, "thead th")).toEqual(SERVICE_LABELS);
		expect(attrs(container, "tbody td", "data-label")).toEqual(SERVICE_LABELS);
		const cells = container.querySelectorAll("tbody td");
		expect(cells[2].textContent).toBe("0 of 1 readymax 1");
		// "0 of 1 ready" is a sentence: text face, left-aligned, like its column head.
		expect(cells[2].className).not.toContain("font-mono");
		expect(cells[2].className).not.toContain("text-right");
		expect(container.querySelectorAll("thead th")[2].className).not.toContain(
			"text-right",
		);
		expect(container.querySelector("thead .sr-only")).toBeNull();
		expect(container.querySelector("tbody [data-act=running]")).not.toBeNull();
		expect(byText("invoice-extractor", cells[0]).getAttribute("title")).toBe(
			"invoice-extractor",
		);
		await click(byRole("button", "Stop…"));
		expect(stopped).toBe(1);
		expect(opened).toBe(0);
		await click(cells[3]);
		expect(opened).toBe(1);
	});

	test("visibility, mode and drift chips", async () => {
		const { container } = await dom.render(
			<At>
				<appChips.VisibilityChip visibility="Offline" />
				<appChips.VisibilityChip visibility="PublicRequestAccess" />
				<appChips.ModeChip mode="online" app="Invoice AI" />
				<appChips.ModeChip mode="offline" />
				<appChips.DriftChip behind={0} />
				<appChips.DriftChip behind={2} />
				<appChips.DriftChip behind={null} />
				<appChips.DriftChip behind={1} staged="v2.4.0" />
				<appChips.VersionCell label="v1.4.0" hash="491e8acf" behind={1} />
				<appChips.VersionCell behind={0} />
			</At>,
		);
		expect(texts(container, "[data-slot=badge]")).toEqual([
			"Local only",
			"Public · on request",
			"Runs online",
			"Offline copy",
			"Newest",
			"2 behind",
			"Unknown",
			"v2.4.0 staged",
			"1 behind",
			"Unknown",
		]);
		expect(attrs(container, "[data-drift]", "data-tone")).toEqual([
			"good",
			"info",
			"unknown",
			"info",
			"info",
			"unknown",
		]);
		const offline = container.querySelector("[data-visibility=Offline]");
		expect(offline?.getAttribute("title")).toBe(
			"Only on this computer. Not synced to your account.",
		);
		expect(container.textContent).toContain("Version unknown");
	});

	test("how it runs strip and the mode explainer mark this app", async () => {
		let explained = 0;
		const { container } = await dom.render(
			<At>
				<HowRunsStrip
					app="Invoice AI"
					visibility="Prototype"
					mode="online"
					newest={{ label: "v1.5.0", hash: "7c2d1e90", at: NOW_S - 60 }}
					onExplain={() => {
						explained += 1;
					}}
				/>
				<ModeExplainer app="Support Portal" mode="offline" />
			</At>,
		);
		byRole("region", "How Invoice AI runs on devices");
		const strip = container.textContent ?? "";
		expect(strip).toContain(
			"Devices run the version you deploy from the hub. Data stays in the cloud, so they need internet.",
		);
		expect(strip).toContain("Newestv1.5.0");
		expect(strip).toContain("· built today ");
		await click(byRole("button", "Online or offline?"));
		expect(explained).toBe(1);
		expect(container.querySelectorAll("td[data-mine]")).toHaveLength(7);
		const explainer = container.textContent ?? "";
		expect(explainer).toContain("This app");
		expect(explainer).toContain("Not for this app");
		expect(explainer).toContain(
			"Support Portal is a local-only app, so every device gets an offline copy.",
		);
		expect(explainer).toContain(
			"Why not online? Support Portal exists only on this computer",
		);
		// The app's base layer gives a bare <p> 28 px leading.
		expect(
			Array.from(container.querySelectorAll("p"), (p) =>
				/\btext-ui\b|leading-\[inherit\]/.test(p.className),
			),
		).toEqual([true, true, true]);
	});

	test("event cell: tile, name, type, pins, how it runs and the new-in chip", async () => {
		const { container, rerender } = await dom.render(
			<At>
				<EventCell
					eventType="http"
					name="Extract invoice"
					eventId="evt_extract_http"
					pin={{ event: "1.5.0", flow: "2.2.0" }}
					runs="Served by the device · checks its web server"
					newIn="v1.5.0"
				/>
			</At>,
		);
		const cell = container.querySelector("[data-event-cell]");
		expect(cell?.textContent).toBe(
			"Extract invoiceEndpointNew in v1.5.0event 1.5.0 · flow 2.2.0Served by the device · checks its web server",
		);
		expect(
			cell?.querySelector("[data-event-tile] svg.lucide-globe"),
		).not.toBeNull();
		expect(classesOf(byText("1.5.0", container))).toContain("font-mono");
		await rerender(
			<At tech>
				<EventCell
					eventType="rest"
					hasPage
					name="Review"
					eventId="evt_invoice_review"
				/>
			</At>,
		);
		const page = container.querySelector("[data-event-cell]");
		expect(page?.textContent).toBe("ReviewPageevt_invoice_review");
		expect(page?.querySelector("svg.lucide-monitor")).not.toBeNull();
		expect(eventIcon("never-heard-of-it")).toBe(eventIcon("quick_action"));
	});

	test("the mode explainer shows an app name with markup characters as it is", async () => {
		const { container } = await dom.render(
			<At>
				<ModeExplainer app="R&D <beta>" mode="online" />
				<ModeExplainer app="A <1>B</1>" mode="offline" />
			</At>,
		);
		const leads = texts(container, "[data-mode-explainer] > p");
		expect(leads).toEqual([
			"R&D <beta> is an online app, so every device runs it online. How an app runs follows the app; it isn't chosen per deploy.",
			"A <1>B</1> is a local-only app, so every device gets an offline copy. How an app runs follows the app; it isn't chosen per deploy.",
		]);
		expect(texts(container, "[data-mode-explainer] > p b")).toEqual([
			"runs it online",
			"offline copy",
		]);
	});

	test("matrix cells render every APP §2.10 state", async () => {
		const unlock = (
			<DvButton size="xs" icon={Lock}>
				Unlock…
			</DvButton>
		);
		const cells = matrixCells(unlock);
		const { container } = await dom.render(
			<At>
				{cells.map((cell) => {
					return <MatrixCell key={cellKey(cell)} {...cell} />;
				})}
			</At>,
		);
		expect(attrs(container, "[data-matrix-cell]", "data-matrix-cell")).toEqual([
			"served",
			"staged",
			"not_served",
			"cant_here",
			"unknown",
			"unknown",
			"no_access",
		]);
		const text = container.textContent ?? "";
		for (const part of MATRIX_TEXTS) expect(text).toContain(part);
		expect(byRole("button", "Deploy here").getAttribute("aria-disabled")).toBe(
			"true",
		);
		const link = container.querySelector("a[href='#svc']");
		expect(link?.textContent).toContain("invoice-extractor");
		expect(link?.querySelector("[title]")?.getAttribute("title")).toBe(
			"invoice-extractor",
		);
		byRole("button", "Activate…");
	});

	test("matrix cells say why a device's status for the app is unknown (every kind the app model produces)", async () => {
		const { container } = await dom.render(
			<At>
				<MatrixCell state="unknown" why="nokeys" />
				<MatrixCell state="unknown" why="offline" since={NOW_S - 3 * 3600} />
				<MatrixCell state="unknown" why="offline" />
				<MatrixCell state="unknown" why="notloaded" />
				<MatrixCell state="unknown" why="error" />
			</At>,
		);
		expect(attrs(container, "[data-matrix-cell]", "data-why")).toEqual([
			"nokeys",
			"offline",
			"offline",
			"notloaded",
			"error",
		]);
		const since = formatTimeOfDay((NOW_S - 3 * 3600) * 1000, {
			locale: "en",
			seconds: false,
		});
		expect(texts(container, "[data-matrix-cell]")).toEqual([
			"No keys on this computer",
			`No status since ${since}`,
			"No status: the device is offline",
			"Not loaded yet",
			"Couldn't read its status",
		]);
	});

	test("diff rows: added, changed with old → new, removed struck", async () => {
		const { container } = await dom.render(
			<DiffRows
				label="Changes to settings v12"
				rows={[
					{ kind: "added", label: "PORT", after: "8081" },
					{ kind: "changed", label: "Instances", before: "1", after: "2" },
					{ kind: "removed", label: "DEBUG", before: "true" },
				]}
			/>,
		);
		byRole("list", "Changes to settings v12");
		expect(attrs(container, "li[data-k]", "data-k")).toEqual([
			"added",
			"changed",
			"removed",
		]);
		const rows = container.querySelectorAll("li[data-k]");
		expect(rows[1].querySelector("s")?.textContent).toBe("1");
		expect(rows[1].textContent).toContain("Changed: Instances");
		expect(rows[1].textContent).toContain("changed to 2");
		expect(rows[2].querySelector("s")?.textContent).toBe("true");
		expect(rows[0].querySelector("s")).toBeNull();
		await dom.cleanup();
		const empty = await dom.render(<DiffRows rows={[]} />);
		expect(empty.container.textContent).toBe("No changes.");
	});
});

const SOURCE_FILES = [
	"requested-actual",
	"paired-pins",
	"rollout-progress",
	"fleet-rollout",
	"attention-list",
	"annunciator",
	"metric",
	"meter",
	"coverage-line",
	"headline",
	"person-chip",
	"checkin-lane",
	"expiry-rail",
	"trust-chain",
	"timeline",
	"log-viewer",
	"command-block",
	"checklist",
	"wizard",
	"tray-item",
	"service-row",
	"app-chips",
	"how-runs",
	"matrix-cell",
	"diff-rows",
	"event-cell",
	"obj-name",
	"day",
];

describe("source hygiene (R12, R13)", () => {
	test("no hard-coded colours, shadows or colour mixing", () => {
		for (const name of SOURCE_FILES) {
			const source = readFileSync(join(import.meta.dir, `${name}.tsx`), "utf8");
			const colour =
				source.match(/#[0-9a-fA-F]{3,8}\b|rgb\(|hsl\(|oklch\(/)?.[0] ?? null;
			const shadow = source.match(/\bshadow-(?!none)[a-z0-9]/)?.[0] ?? null;
			expect({ name, colour, shadow }).toEqual({
				name,
				colour: null,
				shadow: null,
			});
			expect(source.includes("color-mix")).toBe(false);
		}
	});
});
