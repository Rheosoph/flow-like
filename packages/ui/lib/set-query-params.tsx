import { useRouter } from "next/navigation";

export function useSetQueryParams() {
	const router = useRouter();

	return (key: string, value: string | undefined) => {
		const params = new URLSearchParams(window.location.search);
		if (value === undefined || value === null) {
			params.delete(key);
		} else {
			params.set(key, value);
		}
		router.push(`?${params.toString()}`);
	};
}
