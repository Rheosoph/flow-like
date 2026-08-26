const origin = new URL(
	process.env.FLOWBOOK_ORIGIN ?? "https://book.flow-like.com",
);
const failures = [];

function check(condition, message) {
	if (!condition) failures.push(message);
}

async function request(path, init) {
	const url = new URL(path, origin);
	try {
		return await fetch(url, {
			redirect: "follow",
			signal: AbortSignal.timeout(15_000),
			...init,
		});
	} catch (error) {
		failures.push(`${url}: request failed (${error.message})`);
		return undefined;
	}
}

function isMarkdownResponse(response) {
	return (
		response?.headers
			.get("content-type")
			?.split(";", 1)[0]
			.trim()
			.toLowerCase() === "text/markdown"
	);
}

function linkTargets(response, relation) {
	const header = response?.headers.get("link") ?? "";
	const targets = [];

	for (const match of header.matchAll(/<([^>]+)>\s*((?:;\s*[^,]+)*)/g)) {
		const [, target, parameters = ""] = match;
		const rel = parameters.match(/;\s*rel=(?:"([^"]*)"|([^;\s,]+))/i);
		const relations = (rel?.[1] ?? rel?.[2] ?? "")
			.toLowerCase()
			.split(/\s+/)
			.filter(Boolean);
		if (!relations.includes(relation.toLowerCase())) continue;

		try {
			targets.push(new URL(target, response?.url ?? origin).toString());
		} catch {
			failures.push(`Link header: invalid ${relation} target ${target}`);
		}
	}

	return targets;
}

const [
	homeResponse,
	robotsResponse,
	sitemapResponse,
	pdfResponse,
	printResponse,
	llmsResponse,
	llmsFullResponse,
	markdownHomeResponse,
	markdownIntroductionResponse,
] = await Promise.all([
	request("/"),
	request("/robots.txt"),
	request("/sitemap-index.xml"),
	request("/flowbook.pdf", { method: "HEAD" }),
	request("/print/", { method: "HEAD" }),
	request("/llms.txt", { method: "HEAD" }),
	request("/llms-full.txt", { method: "HEAD" }),
	request("/index.md", { method: "HEAD" }),
	request("/introduction/index.md", { method: "HEAD" }),
]);

const home = homeResponse ? await homeResponse.text() : "";
const robots = robotsResponse ? await robotsResponse.text() : "";
const sitemap = sitemapResponse ? await sitemapResponse.text() : "";

for (const [label, response] of [
	["home", homeResponse],
	["robots.txt", robotsResponse],
	["sitemap index", sitemapResponse],
	["PDF", pdfResponse],
	["print view", printResponse],
]) {
	check(response?.ok, `${label}: expected a successful production response`);
}

const canonical = home.match(
	/<link\b[^>]*rel="canonical"[^>]*href="([^"]+)"/i,
)?.[1];
check(
	canonical === origin.toString(),
	`home: canonical is ${canonical ?? "missing"}`,
);
check(
	robots.includes(`Sitemap: ${new URL("/sitemap-index.xml", origin)}`),
	"robots.txt: production sitemap declaration is missing",
);
check(
	sitemap.includes(new URL("/sitemap-0.xml", origin).toString()),
	"sitemap index: production child sitemap is missing",
);
check(
	pdfResponse?.headers.get("content-type")?.includes("application/pdf"),
	"PDF: unexpected Content-Type",
);
check(
	pdfResponse?.headers.get("x-robots-tag")?.toLowerCase().includes("noindex"),
	"PDF: X-Robots-Tag noindex is not active in production",
);
check(
	printResponse?.headers.get("x-robots-tag")?.toLowerCase().includes("noindex"),
	"print view: X-Robots-Tag noindex is not active in production",
);

for (const [label, response] of [
	["llms.txt", llmsResponse],
	["llms-full.txt", llmsFullResponse],
	["home Markdown", markdownHomeResponse],
	["introduction Markdown", markdownIntroductionResponse],
]) {
	check(response?.status === 200, `${label}: expected HTTP 200`);
	check(isMarkdownResponse(response), `${label}: unexpected Content-Type`);
}

for (const [label, response] of [
	["llms.txt", llmsResponse],
	["llms-full.txt", llmsFullResponse],
]) {
	check(
		/\bnoindex\b/i.test(response?.headers.get("x-robots-tag") ?? ""),
		`${label}: X-Robots-Tag noindex is not active in production`,
	);
}

const llmsUrl = new URL("/llms.txt", origin).toString();
for (const [label, response, canonicalPath] of [
	["home Markdown", markdownHomeResponse, "/"],
	["introduction Markdown", markdownIntroductionResponse, "/introduction/"],
]) {
	const expectedCanonical = new URL(canonicalPath, origin).toString();
	const canonicals = linkTargets(response, "canonical");
	const describedBy = linkTargets(response, "describedby");

	check(
		canonicals.length === 1 && canonicals[0] === expectedCanonical,
		`${label}: Link canonical is ${canonicals.join(", ") || "missing"}`,
	);
	check(
		describedBy.length === 1 && describedBy[0] === llmsUrl,
		`${label}: Link describedby is ${describedBy.join(", ") || "missing"}`,
	);
}

if (failures.length > 0) {
	console.error(`Live SEO smoke test failed with ${failures.length} issue(s):`);
	for (const failure of failures) console.error(`- ${failure}`);
	process.exit(1);
}

console.log(`Live SEO smoke test passed for ${origin.origin}.`);
