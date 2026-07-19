export function getPackageOverviewHref(
	searchParams: Pick<URLSearchParams, "toString">,
): string {
	const params = new URLSearchParams(searchParams.toString());
	params.delete("id");
	params.delete("purchase");
	const query = params.toString();
	return query ? `/store/packages?${query}` : "/store/packages";
}
