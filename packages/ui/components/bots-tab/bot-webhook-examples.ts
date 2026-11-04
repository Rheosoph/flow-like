function shellQuote(value: string): string {
	return `'${value.replaceAll("'", "'\"'\"'")}'`;
}

export function authenticatedWebhookUrl(url: string, secret: string): string {
	const fragmentIndex = url.indexOf("#");
	const beforeFragment = fragmentIndex < 0 ? url : url.slice(0, fragmentIndex);
	const fragment = fragmentIndex < 0 ? "" : url.slice(fragmentIndex);
	const queryIndex = beforeFragment.indexOf("?");
	const path =
		queryIndex < 0 ? beforeFragment : beforeFragment.slice(0, queryIndex);
	const query = new URLSearchParams(
		queryIndex < 0 ? "" : beforeFragment.slice(queryIndex + 1),
	);
	query.set("secret", secret);
	return `${path}?${query}${fragment}`;
}

export function webhookCurlExamples(url: string, secret: string) {
	const payload = JSON.stringify({
		event: "example",
		message: "Hello from a webhook",
	});
	const body = [
		"  --header 'Content-Type: application/json'",
		`  --data-raw ${shellQuote(payload)}`,
	];
	return {
		header: [
			`curl --fail-with-body ${shellQuote(url)}`,
			`  --header ${shellQuote(`X-Flow-Like-Webhook-Secret: ${secret}`)}`,
			...body,
		].join(" \\\n"),
		url: [
			`curl --fail-with-body ${shellQuote(authenticatedWebhookUrl(url, secret))}`,
			...body,
		].join(" \\\n"),
	};
}
