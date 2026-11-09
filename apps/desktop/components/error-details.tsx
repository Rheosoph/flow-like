"use client";

import { normalizeError } from "@flow-like/flow-like-ui/lib/telemetry/errors";
import { useTranslation } from "@flow-like/locales";

/** Remains usable when startup fails before the app's providers mount. */
export function ErrorDetails({ error }: Readonly<{ error: unknown }>) {
	const { t } = useTranslation("common");
	const { kind, value } = normalizeError(error);

	return (
		<details style={{ maxWidth: "100%", width: "36rem" }}>
			<summary style={{ cursor: "pointer", fontSize: "0.875rem" }}>
				{t("details", "Details")}
			</summary>
			<pre
				style={{
					marginTop: "12px",
					fontSize: "0.75rem",
					whiteSpace: "pre-wrap",
					overflowWrap: "anywhere",
					userSelect: "text",
				}}
			>
				{`${kind}: ${value}`}
			</pre>
		</details>
	);
}
