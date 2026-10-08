// @vitest-environment happy-dom
import { type ButtonHTMLAttributes, type ReactNode, act } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { SignInRequired } from "./sign-in-required";

const { signinRedirect } = vi.hoisted(() => ({ signinRedirect: vi.fn() }));

vi.mock("react-oidc-context", () => ({
	useAuth: () => ({ signinRedirect }),
}));
vi.mock("@flow-like/locales", () => ({
	useTranslation: () => ({ t: (_key: string, fallback: string) => fallback }),
}));
vi.mock("@flow-like/flow-like-ui/components/ui/button", () => ({
	Button: ({
		asChild,
		children,
		size: _size,
		variant: _variant,
		...props
	}: ButtonHTMLAttributes<HTMLButtonElement> & {
		asChild?: boolean;
		size?: string;
		variant?: string;
	}) => (asChild ? children : <button {...props}>{children}</button>),
}));
vi.mock("next/image", () => ({ default: () => null }));
vi.mock("next/link", () => ({
	default: ({ children }: { children: ReactNode }) => children,
}));

let root: Root;
let container: HTMLDivElement;

beforeEach(() => {
	(
		globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
	).IS_REACT_ACT_ENVIRONMENT = true;
	window.history.replaceState(null, "", "/library/config?app=app-1#boards");
	signinRedirect.mockReset();
	container = document.createElement("div");
	document.body.appendChild(container);
	root = createRoot(container);
});

afterEach(async () => {
	await act(async () => root.unmount());
	container.remove();
});

it("allows retry after OIDC resolves a failed redirect with null and preserves the return URL", async () => {
	let finishRedirect!: (result: null) => void;
	signinRedirect.mockImplementationOnce(
		() =>
			new Promise<null>((resolve) => {
				finishRedirect = resolve;
			}),
	);
	signinRedirect.mockResolvedValue(null);
	await act(async () => root.render(<SignInRequired />));
	const button = Array.from(container.querySelectorAll("button")).find(
		(candidate) => candidate.textContent?.includes("Sign In to Continue"),
	);
	expect(button).toBeDefined();
	if (!button) throw new Error("Sign-in button was not rendered");

	act(() => button.click());
	expect(button.disabled).toBe(true);
	expect(signinRedirect).toHaveBeenCalledExactlyOnceWith({
		url_state: "/library/config?app=app-1#boards",
	});

	await act(async () => finishRedirect(null));
	expect(button.disabled).toBe(false);
	expect(button.textContent).toContain("Sign In to Continue");

	await act(async () => button.click());
	expect(signinRedirect).toHaveBeenCalledTimes(2);
	expect(signinRedirect).toHaveBeenLastCalledWith({
		url_state: "/library/config?app=app-1#boards",
	});
	expect(button.disabled).toBe(false);
});
