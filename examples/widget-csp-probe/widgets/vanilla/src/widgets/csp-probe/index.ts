import { mountFlowWidget } from "@flow-like/widget-sdk";
import {
	isHistoryBackStep,
	navigationMarker,
	readDocumentInfo,
	resolveExpectation,
} from "../../lib/location";
import {
	BLOCKED_EXPECTATION,
	HISTORY_BACK_DELAY_MS,
	NAVIGATION_CASES,
	NAVIGATION_WAIT_MS,
	type NavigationCaseId,
	historyBackStayed,
	startNavigation,
} from "../../lib/navigation";
import { watchViolations } from "../../lib/network";
import { reachedReport, whenConnected } from "../../lib/reached";
import {
	type ProbeCheck,
	type ProbeChecks,
	type ProbePhase,
	type ProbeReport,
	check,
	createReport,
} from "../../lib/report";
import { runSuite } from "../../lib/suite";
import { ResultsTable, renderBanner, summaryText } from "../../lib/view";
import widget from "./widget.config";

const bridge = mountFlowWidget(widget);
const info = readDocumentInfo();

const currentExpectation = () =>
	resolveExpectation(
		bridge.$props.get().expectation,
		info,
		bridge.$capabilities.get().preview === true,
	);

const buildReport = (phase: ProbePhase, checks: ProbeChecks): ProbeReport =>
	createReport({
		widgetId: widget.id,
		phase,
		expectation: currentExpectation(),
		document: info.label,
		checks,
	});

let lastReport = buildReport("suite", {});
bridge.onQuery("getReport", () => lastReport);

const root = document.getElementById("root");
if (root) {
	const main = document.createElement("main");
	const title = document.createElement("h1");
	title.textContent = "Widget CSP probe";
	const meta = document.createElement("p");
	meta.className = "meta";
	const summary = document.createElement("p");
	summary.className = "summary";

	const runButton = document.createElement("button");
	runButton.type = "button";
	runButton.className = "primary";
	runButton.textContent = "Run checks";

	const caseSelect = document.createElement("select");
	caseSelect.setAttribute("aria-label", "Navigation case");
	for (const navigationCase of NAVIGATION_CASES) {
		const option = document.createElement("option");
		option.value = navigationCase.id;
		option.textContent = navigationCase.label;
		caseSelect.append(option);
	}
	const navigateButton = document.createElement("button");
	navigateButton.type = "button";
	navigateButton.className = "danger";
	navigateButton.textContent = "Attempt navigation";

	const runControls = document.createElement("div");
	runControls.className = "controls";
	runControls.append(runButton);
	const navigationControls = document.createElement("div");
	navigationControls.className = "controls";
	navigationControls.append(caseSelect, navigateButton);
	const navigationHint = document.createElement("p");
	navigationHint.className = "hint";
	navigationHint.textContent =
		"A navigation attempt can replace this document. Remount the widget before the next attempt. An engine error page in this frame counts as blocked.";

	const table = new ResultsTable();
	const frameSlot = document.createElement("div");
	frameSlot.className = "frame-slot";

	main.append(
		title,
		meta,
		runControls,
		summary,
		navigationControls,
		navigationHint,
		table.root,
		frameSlot,
	);
	root.append(main);

	const renderMeta = () => {
		meta.textContent = `${info.label} · expecting ${currentExpectation()} · ${bridge.$mode.get()}`;
	};
	bridge.$mode.subscribe(renderMeta);
	bridge.$props.subscribe(renderMeta);
	bridge.$capabilities.subscribe(renderMeta);

	const publish = (report: ProbeReport) => {
		lastReport = report;
		summary.textContent = summaryText(report);
		bridge.emit("result", report);
	};
	const publishNavigation = (id: string, result: ProbeCheck) => {
		table.set(id, result);
		publish(buildReport("navigation", { [id]: result }));
	};

	let busy = false;
	const setBusy = (next: boolean) => {
		busy = next;
		runButton.disabled = next;
		navigateButton.disabled = next;
	};

	const runChecks = async () => {
		if (busy) return;
		setBusy(true);
		table.clear();
		summary.textContent = "Running…";
		const violations = watchViolations(document);
		try {
			const checks = await runSuite({
				csp: widget.csp,
				expectation: currentExpectation(),
				frameSlot,
				violations,
				onCheck: (id, result) => table.set(id, result),
			});
			publish(buildReport("suite", checks));
		} finally {
			violations.dispose();
			setBusy(false);
		}
	};

	const attemptNavigation = (caseId: NavigationCaseId) => {
		if (busy) return;
		setBusy(true);
		const start = startNavigation(caseId, info, (action) =>
			publishNavigation(
				caseId,
				check(
					"review",
					BLOCKED_EXPECTATION,
					`${action}; a follow-up report or an engine error page in this frame decides the result`,
				),
			),
		);
		if (start.kind === "resolved") {
			publishNavigation(caseId, start.check);
			setBusy(false);
			return;
		}
		setTimeout(() => {
			publishNavigation(caseId, start.stayed);
			setBusy(false);
		}, NAVIGATION_WAIT_MS);
	};

	runButton.addEventListener("click", () => void runChecks());
	navigateButton.addEventListener("click", () =>
		attemptNavigation(caseSelect.value as NavigationCaseId),
	);

	const marker = navigationMarker();
	if (marker !== null) {
		renderBanner(
			main,
			`NAVIGATION REACHED: ${marker} loaded ${info.label}. This check failed.`,
		);
		setBusy(true);
		void whenConnected(bridge).then(() => {
			const report = reachedReport(widget.id, marker, info);
			table.setAll(report.checks);
			publish(report);
		});
	} else if (isHistoryBackStep()) {
		setBusy(true);
		void whenConnected(bridge).then(() => {
			if (bridge.$mode.get() !== "hosted") {
				setBusy(false);
				return;
			}
			publishNavigation(
				"nav.historyBack",
				check(
					"review",
					BLOCKED_EXPECTATION,
					"calling history.back(); a follow-up report or an engine error page in this frame decides the result",
				),
			);
			setTimeout(() => history.back(), HISTORY_BACK_DELAY_MS);
			setTimeout(() => {
				publishNavigation("nav.historyBack", historyBackStayed());
				setBusy(false);
			}, HISTORY_BACK_DELAY_MS + NAVIGATION_WAIT_MS);
		});
	} else {
		void whenConnected(bridge).then(() => {
			if (bridge.$props.get().autoRun) void runChecks();
		});
	}
}
