import { afterAll, afterEach, describe, expect, test } from "bun:test";
import { useState } from "react";
import type { IEvent } from "../../../lib/schema/flow/event";
import {
	byRole,
	click,
	inPortal,
	installDom,
} from "../devices/testing/dom-harness";

const dom = installDom();
const { toast } = await import("sonner");
const { NewEventDialog } = await import("./new-event-dialog");

afterEach(dom.cleanup);
afterAll(dom.restore);

const saved = { id: "evt_1", name: "Visitor welcome" } as IEvent;

function Host({
	log,
	deployed,
}: Readonly<{ log: boolean[]; deployed?: string[] }>) {
	const [open, setOpen] = useState(true);
	return (
		<>
			<button type="button" onClick={() => setOpen(true)}>
				reopen
			</button>
			<NewEventDialog
				open={open}
				onOpenChange={(next) => {
					log.push(next);
					setOpen(next);
				}}
				onDeployed={() => deployed?.push("deployed")}
			>
				{(shell) => (
					<div>
						<button type="button" onClick={() => shell.onSavedChange(saved)}>
							save
						</button>
						<button type="button" onClick={() => shell.onSavedChange(null)}>
							unmount form
						</button>
						<button type="button" onClick={() => shell.onBusyChange(true)}>
							busy
						</button>
						<button type="button" onClick={() => shell.onDeployedChange(true)}>
							deployed
						</button>
						<button type="button" onClick={shell.onDeploymentComplete}>
							finish
						</button>
						<button type="button" onClick={shell.onCancel}>
							cancel
						</button>
					</div>
				)}
			</NewEventDialog>
		</>
	);
}

const closeButtons = () =>
	document.querySelectorAll("[data-slot='dialog-close']").length;
const toasts = () =>
	toast
		.getHistory()
		.flatMap((entry) => ("title" in entry ? [entry.title] : []));

describe("New event dialog shell", () => {
	test("retitles itself once an event is saved", async () => {
		await dom.render(<Host log={[]} />);
		const dialog = inPortal("dialog");
		expect(dialog.textContent).toContain("New event");
		expect(dialog.textContent).toContain(
			"Choose what starts your flow and where it runs.",
		);
		await click(byRole("button", "save"));
		expect(dialog.textContent).toContain("Deploy Visitor welcome");
		expect(dialog.textContent).toContain(
			"Event saved. Finish the deployment to your devices.",
		);
	});

	test("closing after the save, before a deploy finished, says where to finish", async () => {
		const log: boolean[] = [];
		await dom.render(<Host log={log} />);
		const before = toasts().length;
		await click(byRole("button", "save"));
		await click(byRole("button", "cancel"));
		expect(log).toEqual([false]);
		expect(toasts().slice(before)).toEqual([
			"Visitor welcome was saved. Deploy it from its Runs on column.",
		]);
	});

	test("closing without a saved event, or after a finished deploy, raises no notice", async () => {
		const before = toasts().length;
		await dom.render(<Host log={[]} />);
		await click(byRole("button", "cancel"));
		await dom.cleanup();

		const deployed: string[] = [];
		await dom.render(<Host log={[]} deployed={deployed} />);
		await click(byRole("button", "save"));
		await click(byRole("button", "finish"));
		expect(deployed).toEqual(["deployed"]);
		expect(toasts().slice(before)).toEqual([]);
	});

	test("closing after a deploy succeeded raises no notice, even though Done was not pressed", async () => {
		const log: boolean[] = [];
		await dom.render(<Host log={log} />);
		const before = toasts().length;
		await click(byRole("button", "save"));
		await click(byRole("button", "deployed"));
		await click(byRole("button", "cancel"));
		expect(log).toEqual([false]);
		expect(toasts().slice(before)).toEqual([]);
	});

	test("a deploy reported for one event does not silence the notice for the next", async () => {
		const before = toasts().length;
		await dom.render(<Host log={[]} />);
		await click(byRole("button", "save"));
		await click(byRole("button", "deployed"));
		await click(byRole("button", "cancel"));
		await click(byRole("button", "reopen"));
		await click(byRole("button", "save"));
		await click(byRole("button", "cancel"));
		expect(toasts().slice(before)).toEqual([
			"Visitor welcome was saved. Deploy it from its Runs on column.",
		]);
	});

	test("a busy step blocks dismissal, and the form going away frees the dialog", async () => {
		await dom.render(<Host log={[]} />);
		await click(byRole("button", "save"));
		await click(byRole("button", "busy"));
		expect(closeButtons()).toBe(0);
		await click(byRole("button", "unmount form"));
		expect(inPortal("dialog").textContent).toContain("New event");
		expect(closeButtons()).toBe(1);
	});
});
