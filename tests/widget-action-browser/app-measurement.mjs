/** Installs measurements in the real application without replacing its transport or UI. */
export function installAppMeasurement(config) {
	const state = { navigation: null, result: null, active: null };
	window.__flowLikeAppBenchmark = state;
	const visible = (element) =>
		element instanceof HTMLElement &&
		element.getClientRects().length > 0 &&
		getComputedStyle(element).visibility !== "hidden";
	const actionElement = (action) =>
		[...document.querySelectorAll(action.selector)].find(
			(element) => !action.text || element.textContent?.includes(action.text),
		);
	state.actionElement = (index) => actionElement(config.actions[index]);
	state.resetElement = () => actionElement(config.reset);
	const actionable = (action) => {
		const element = actionElement(action);
		return (
			visible(element) &&
			!element.disabled &&
			!element.closest("[data-a2ui-action-pending]") &&
			getComputedStyle(element).pointerEvents !== "none"
		);
	};
	const nextFrames = (callback) =>
		requestAnimationFrame(() => requestAnimationFrame(callback));
	const history = (action) => {
		const panel = document.querySelector(action.historySelector);
		const text = panel?.textContent ?? "";
		const rows = panel
			? [...panel.querySelectorAll(action.rowSelector ?? "tbody tr")].filter(
					visible,
				)
			: [];
		return {
			text,
			rows: rows.length,
			matches:
				visible(panel) &&
				text.includes(action.expectedText) &&
				!config.actions.some(
					(other) => other !== action && text.includes(other.expectedText),
				) &&
				(action.expectedRows === undefined ||
					rows.length === action.expectedRows),
		};
	};
	const ready = () => {
		if (state.navigation) return;
		try {
			if (
				config.readySelector &&
				!visible(document.querySelector(config.readySelector))
			)
				return;
			if (!config.actions.every(actionable)) return;
			state.navigation = {
				cardsDomMs: performance.now(),
				visibility: document.visibilityState,
			};
			nextFrames(() => {
				state.navigation.cardsFrameOpportunityMs = performance.now();
			});
		} catch {
			state.navigation = { error: "Invalid readiness selector" };
		}
	};
	const navigationObserver = new MutationObserver(ready);
	navigationObserver.observe(document, {
		childList: true,
		subtree: true,
		attributes: true,
	});
	window.addEventListener("DOMContentLoaded", ready, { once: true });
	const navigationTimer = setInterval(() => {
		ready();
		if (state.navigation) {
			clearInterval(navigationTimer);
			navigationObserver.disconnect();
		}
	}, 25);

	state.arm = (actionIndex, reset = false) => {
		if (state.active)
			throw new Error("Previous action is still being measured");
		if (document.visibilityState !== "visible")
			throw new Error("Keep the benchmark tab visible");
		const action = reset ? config.reset : config.actions[actionIndex];
		const target = () =>
			reset
				? {
						matches: config.actions.every(actionable),
						text: "",
						rows: config.actions.length,
					}
				: history(action);
		const domMetric = reset ? "clickToCardsDomMs" : "clickToHistoryDomMs";
		const frameMetric = reset
			? "clickToCardsFrameOpportunityMs"
			: "clickToHistoryFrameOpportunityMs";
		if (!actionable(action))
			throw new Error("The action is not ready to click");
		const before = target();
		if (before.matches)
			throw new Error(
				reset
					? "The configured cards are already ready"
					: "History already matches this action; alternate distinct people",
			);
		const previousTraces = new Set(window.__flowLikeRunTimings ?? []);
		const trial = {
			actionIndex,
			status: "ok",
			visibility: document.visibilityState,
		};
		state.result = null;
		state.active = trial;
		let clickAt;
		let observer;
		let poll;
		let timeout;
		const finish = (error) => {
			if (state.active !== trial) return;
			observer?.disconnect();
			clearInterval(poll);
			clearTimeout(timeout);
			document.removeEventListener("click", click, true);
			if (error) {
				trial.status = "error";
				trial.error = error;
			}
			if (document.visibilityState !== "visible") {
				trial.status = "error";
				trial.error = "The tab became hidden during measurement";
			}
			state.active = null;
			state.result = trial;
		};
		const inspect = () => {
			if (clickAt === undefined || state.active !== trial) return;
			const current = target();
			if (
				trial[domMetric] === undefined &&
				current.matches &&
				(reset || current.text !== before.text)
			) {
				trial[domMetric] = performance.now() - clickAt;
				if (reset) trial.readyActions = current.rows;
				else trial.visibleRows = current.rows;
				nextFrames(() => {
					if (state.active !== trial) return;
					if (!target().matches)
						return finish(
							"Expected content changed before its frame opportunity",
						);
					trial[frameMetric] = performance.now() - clickAt;
					inspect();
				});
			}
			const trace = window.__flowLikeRunTimings?.findLast(
				(item) =>
					!previousTraces.has(item) &&
					["widget action", "workflow action"].includes(item.label),
			);
			if (trace && trial.actionTimings === undefined) {
				trial.actionTimings = trace;
				trial.clickToSettledMs = performance.now() - clickAt;
				trial.handlerToSettledMs = trace.totalMs;
				const mark = (name) =>
					trace.marks?.find((item) => item.name === name)?.atMs;
				trial.handlerToDispatchMs = mark("dispatch");
				trial.handlerToPatchMs = mark("first_a2ui_update_received");
			}
			if (
				trial[frameMetric] !== undefined &&
				(trial.actionTimings || config.requireActionTrace === false)
			)
				finish();
		};
		const click = (event) => {
			if (
				clickAt !== undefined ||
				!(event.target instanceof Element) ||
				!actionElement(action)?.contains(event.target)
			)
				return;
			clickAt = performance.now();
			trial.clickAtNavigationMs = clickAt;
			trial.syntheticBrowserInputTrusted = event.isTrusted;
			queueMicrotask(inspect);
		};
		document.addEventListener("click", click, true);
		observer = new MutationObserver(inspect);
		observer.observe(document, {
			subtree: true,
			childList: true,
			characterData: true,
			attributes: true,
		});
		poll = setInterval(inspect, 10);
		timeout = setTimeout(
			() =>
				finish(
					clickAt === undefined
						? "The configured action did not receive a click"
						: "Expected content or action handler settlement did not arrive before the timeout",
				),
			config.timeoutMs,
		);
	};
}

export function requestCategory(rawUrl, method) {
	let path;
	try {
		path = new URL(rawUrl).pathname;
	} catch {
		return null;
	}
	const category = /\/pages\/bootstrap\/?$/.test(path)
		? "bootstrap"
		: /\/prerun\/?$/.test(path)
			? "prerun"
			: /\/invoke\/?$/.test(path)
				? "invoke"
				: /\/widgets\/?$/.test(path)
					? "widgets"
					: /\/api\//.test(path)
						? "other_api"
						: null;
	return category && method === "OPTIONS" ? `${category}_preflight` : category;
}
