import { type FormEvent, useId, useState } from "react";

import "./IncidentDeskDemo.css";

type Impact = "production-stopped" | "degraded" | "limited";

interface IncidentResult {
	severity: "SEV-1" | "SEV-2" | "SEV-3";
	team: string;
	runbook: string;
}

const initialResult: IncidentResult = {
	severity: "SEV-1",
	team: "payments-on-call",
	runbook: "runbooks/payments.md",
};

function previewTriage(
	systemId: string,
	report: string,
	impact: Impact,
): IncidentResult {
	const system = systemId.trim().toLowerCase();
	const normalizedReport = report.trim().toLowerCase();
	const severity =
		impact === "production-stopped" ||
		normalizedReport.includes("production is on hold") ||
		normalizedReport.includes("outage")
			? "SEV-1"
			: impact === "degraded"
				? "SEV-2"
				: "SEV-3";
	const payments = system.includes("payment") || system.includes("checkout");

	return {
		severity,
		team: payments ? "payments-on-call" : "platform-on-call",
		runbook: payments ? "runbooks/payments.md" : "runbooks/general.md",
	};
}

export default function IncidentDeskDemo() {
	const id = useId();
	const [systemId, setSystemId] = useState("PAYMENTS-EU");
	const [report, setReport] = useState(
		"Production is on hold after checkout requests started failing.",
	);
	const [impact, setImpact] = useState<Impact>("production-stopped");
	const [result, setResult] = useState<IncidentResult>(initialResult);
	const [previewCount, setPreviewCount] = useState(0);

	function handleSubmit(event: FormEvent<HTMLFormElement>) {
		event.preventDefault();
		setResult(previewTriage(systemId, report, impact));
		setPreviewCount((count) => count + 1);
	}

	return (
		<section
			className="not-content incident-desk"
			aria-labelledby={`${id}-title`}
		>
			<header className="incident-desk__header">
				<div>
					<p className="incident-desk__eyebrow">Embedded React prototype</p>
					<h3 id={`${id}-title`}>Incident Desk</h3>
					<p>
						Route an interruption to the people and runbook that can resolve it.
					</p>
				</div>
				<span className="incident-desk__runtime">
					<span aria-hidden="true" /> Web · Remote
				</span>
			</header>

			<div className="incident-desk__layout">
				<form className="incident-desk__form" onSubmit={handleSubmit}>
					<div className="incident-desk__field">
						<label htmlFor={`${id}-system`}>Affected system</label>
						<input
							id={`${id}-system`}
							name="systemId"
							value={systemId}
							onChange={(event) => setSystemId(event.target.value)}
							placeholder="PAYMENTS-EU"
							required
						/>
					</div>

					<div className="incident-desk__field">
						<label htmlFor={`${id}-impact`}>Business impact</label>
						<select
							id={`${id}-impact`}
							name="impact"
							value={impact}
							onChange={(event) => setImpact(event.target.value as Impact)}
						>
							<option value="production-stopped">Production stopped</option>
							<option value="degraded">Service degraded</option>
							<option value="limited">Limited impact</option>
						</select>
					</div>

					<div className="incident-desk__field incident-desk__field--wide">
						<label htmlFor={`${id}-report`}>What is happening?</label>
						<textarea
							id={`${id}-report`}
							name="report"
							value={report}
							onChange={(event) => setReport(event.target.value)}
							rows={4}
							required
						/>
					</div>

					<div className="incident-desk__submit">
						<button type="submit">Triage incident</button>
						<p>
							This book prototype calculates locally. It does not invoke a Flow.
						</p>
					</div>
				</form>

				<aside className="incident-desk__result" aria-live="polite">
					<div className="incident-desk__result-heading">
						<span>Structured result</span>
						<span className="incident-desk__mock">Mock</span>
					</div>

					<div
						className="incident-desk__severity"
						data-severity={result.severity}
					>
						<span>Severity</span>
						<strong>{result.severity}</strong>
					</div>

					<dl>
						<div>
							<dt>Responsible team</dt>
							<dd>{result.team}</dd>
						</div>
						<div>
							<dt>Runbook</dt>
							<dd>
								<code>{result.runbook}</code>
							</dd>
						</div>
					</dl>

					<div className="incident-desk__binding">
						<span>Page action</span>
						<code>workflow_event → triageQuickAction</code>
					</div>

					<p className="incident-desk__announcement">
						{previewCount === 0
							? "Example output is ready. Change the form to preview another response."
							: `Local preview recalculated ${previewCount} ${previewCount === 1 ? "time" : "times"}.`}
					</p>
				</aside>
			</div>
		</section>
	);
}
