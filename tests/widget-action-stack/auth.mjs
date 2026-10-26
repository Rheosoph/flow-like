import {
	generateKeyPairSync,
	createPrivateKey,
	createPublicKey,
	sign,
	verify,
	X509Certificate,
} from "node:crypto";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import https from "node:https";
import http from "node:http";
import { resolve } from "node:path";
import { execFileSync } from "node:child_process";

// Credentials belong only to this isolated fixture. API signature validation is unchanged.
const [command = "prepare", directory = "/tmp/flow-widget-stack"] =
	process.argv.slice(2);
if (["help", "--help"].includes(command)) {
	console.log(
		"node tests/widget-action-stack/auth.mjs prepare|serve [STATE_DIRECTORY]",
	);
	process.exit(0);
}
const state = resolve(directory);
const fixture = JSON.parse(readFileSync(`${state}/stack.json`, "utf8"));
if (fixture.fixture !== "flow-like-widget-action-stack-v1")
	throw new Error("Unknown fixture directory");
const issuer = "https://127.0.0.1:9440";
// UserInfo uses a separate native HTTP client, which does not load the fixture CA.
const userInfoOrigin = "http://127.0.0.1:9441";
const client = "flow-widget-benchmark";
const subject = "widget-bench-user";
const write = (name, value) =>
	writeFileSync(`${state}/${name}`, value, { mode: 0o600 });
const json = (name, value) =>
	write(name, `${JSON.stringify(value, null, 2)}\n`);
const certificateCurrent = (name) => {
	try {
		const certificate = new X509Certificate(readFileSync(`${state}/${name}`));
		return (
			Date.parse(certificate.validFrom) <= Date.now() &&
			Date.parse(certificate.validTo) > Date.now() + 24 * 3600 * 1000
		);
	} catch {
		return false;
	}
};

if (command === "prepare") {
	if (!existsSync(`${state}/oidc-signing.pem`)) {
		const { privateKey } = generateKeyPairSync("rsa", { modulusLength: 2048 });
		write(
			"oidc-signing.pem",
			privateKey.export({ type: "pkcs8", format: "pem" }),
		);
	}
	if (
		!certificateCurrent("oidc-ca.pem") ||
		!certificateCurrent("oidc-cert.pem") ||
		!existsSync(`${state}/oidc-ca-key.pem`) ||
		!existsSync(`${state}/oidc-tls.pem`)
	) {
		execFileSync(
			"openssl",
			[
				"req",
				"-x509",
				"-newkey",
				"rsa:2048",
				"-nodes",
				"-days",
				"30",
				"-keyout",
				`${state}/oidc-ca-key.pem`,
				"-out",
				`${state}/oidc-ca.pem`,
				"-subj",
				"/CN=Flow widget benchmark local root",
				"-addext",
				"basicConstraints=critical,CA:TRUE",
				"-addext",
				"keyUsage=critical,keyCertSign,cRLSign",
			],
			{ stdio: "ignore" },
		);
		write(
			"oidc-server.ext",
			"subjectAltName=IP:127.0.0.1,DNS:localhost\nbasicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature,keyEncipherment\nextendedKeyUsage=serverAuth\n",
		);
		execFileSync(
			"openssl",
			[
				"req",
				"-new",
				"-newkey",
				"rsa:2048",
				"-nodes",
				"-keyout",
				`${state}/oidc-tls.pem`,
				"-out",
				`${state}/oidc-server.csr`,
				"-subj",
				"/CN=localhost",
			],
			{ stdio: "ignore" },
		);
		execFileSync(
			"openssl",
			[
				"x509",
				"-req",
				"-in",
				`${state}/oidc-server.csr`,
				"-CA",
				`${state}/oidc-ca.pem`,
				"-CAkey",
				`${state}/oidc-ca-key.pem`,
				"-CAcreateserial",
				"-days",
				"30",
				"-out",
				`${state}/oidc-cert.pem`,
				"-extfile",
				`${state}/oidc-server.ext`,
			],
			{ stdio: "ignore" },
		);
	}
	const key = createPrivateKey(readFileSync(`${state}/oidc-signing.pem`));
	const publicKey = createPublicKey(key).export({ format: "jwk" });
	json("oidc-jwks.json", {
		keys: [{ ...publicKey, kid: "widget-bench", use: "sig", alg: "RS256" }],
	});
	const issued = Math.floor(Date.now() / 1000);
	const claims = {
		iss: issuer,
		sub: subject,
		aud: client,
		client_id: client,
		iat: issued,
		exp: issued + 8 * 3600,
		email: "benchmark@example.invalid",
		email_verified: true,
		name: "Benchmark User",
		preferred_username: "widget-benchmark",
	};
	const base64 = (value) =>
		Buffer.from(JSON.stringify(value)).toString("base64url");
	const unsigned = `${base64({ alg: "RS256", typ: "JWT", kid: "widget-bench" })}.${base64(claims)}`;
	const token = `${unsigned}.${sign("RSA-SHA256", Buffer.from(unsigned), key).toString("base64url")}`;
	const user = JSON.stringify({
		id_token: token,
		access_token: token,
		token_type: "Bearer",
		scope: "openid profile email",
		profile: claims,
		expires_at: claims.exp,
	});
	json("storage-state.json", {
		origins: ["http://127.0.0.1:3440", "http://localhost:3440"].map(
			(origin) => ({
				origin,
				localStorage: [{ name: `oidc.user:${issuer}:${client}`, value: user }],
			}),
		),
	});
	write("access-token", token);
	const platform = JSON.parse(
		readFileSync(
			new URL(
				"../../apps/backend/kubernetes/flow-like.config.example.json",
				import.meta.url,
			),
			"utf8",
		),
	);
	platform.name = "Local widget benchmark";
	platform.domain = new URL(fixture.environment.API_URL).host;
	platform.app = platform.web = "127.0.0.1:3440";
	platform.hubs = [];
	platform.signaling = [];
	platform.oauth_providers = {};
	platform.features.model_hosting = false;
	platform.authentication = {
		variant: "openid",
		openid: {
			authority: issuer,
			issuer,
			client_id: client,
			audience: client,
			redirect_uri: "http://127.0.0.1:3440/callback",
			post_logout_redirect_uri: "http://127.0.0.1:3440/",
			response_type: "code",
			scope: "openid profile email",
			jwks_url: `${issuer}/jwks`,
			user_info_url: `${userInfoOrigin}/userinfo`,
			discovery_url: `${issuer}/.well-known/openid-configuration`,
		},
	};
	Object.assign(platform.tiers.FREE, {
		max_runtime_ms: -1,
		max_concurrent_executions: 64,
	});
	json("platform.json", platform);
	write(
		"user.sql",
		`INSERT INTO "User" (id,email,name,"preferredUsername","tutorialCompleted","updatedAt")
VALUES ('${subject}','benchmark@example.invalid','Benchmark User','widget-benchmark',true,now())
ON CONFLICT (id) DO NOTHING;\n`,
	);
	console.log(
		`Prepared local issuer, platform settings and browser session in ${state}.`,
	);
} else if (command === "serve") {
	const jwks = readFileSync(`${state}/oidc-jwks.json`);
	const publicKey = createPublicKey(readFileSync(`${state}/oidc-signing.pem`));
	const identity = (authorization) => {
		try {
			const [scheme, token] = (authorization ?? "").split(" ");
			if (scheme !== "Bearer" || !token) return null;
			const parts = token.split(".");
			if (parts.length !== 3) return null;
			const [header, payload, signature] = parts;
			const algorithm = JSON.parse(Buffer.from(header, "base64url"));
			if (algorithm.alg !== "RS256" || algorithm.kid !== "widget-bench")
				return null;
			if (
				!verify(
					"RSA-SHA256",
					Buffer.from(`${header}.${payload}`),
					publicKey,
					Buffer.from(signature, "base64url"),
				)
			)
				return null;
			const claims = JSON.parse(Buffer.from(payload, "base64url"));
			if (
				claims.iss !== issuer ||
				claims.aud !== client ||
				claims.sub !== subject ||
				!Number.isFinite(claims.exp) ||
				claims.exp <= Date.now() / 1000
			)
				return null;
			return claims;
		} catch {
			return null;
		}
	};
	const handler = (req, res) => {
		res.setHeader("Content-Type", "application/json");
		res.setHeader("Cache-Control", "no-store");
		if (req.url === "/jwks") return res.end(jwks);
		if (req.url === "/userinfo" && req.method === "GET") {
			const claims = identity(req.headers.authorization);
			if (!claims) {
				res.setHeader("WWW-Authenticate", "Bearer");
				return res
					.writeHead(401)
					.end(JSON.stringify({ error: "invalid_token" }));
			}
			return res.end(
				JSON.stringify({
					sub: claims.sub,
					email: claims.email,
					email_verified: claims.email_verified,
					name: claims.name,
					preferred_username: claims.preferred_username,
				}),
			);
		}
		if (req.url === "/.well-known/openid-configuration")
			return res.end(
				JSON.stringify({
					issuer,
					jwks_uri: `${issuer}/jwks`,
					userinfo_endpoint: `${userInfoOrigin}/userinfo`,
					authorization_endpoint: `${issuer}/authorize`,
					token_endpoint: `${issuer}/token`,
					response_types_supported: ["code"],
					subject_types_supported: ["public"],
					id_token_signing_alg_values_supported: ["RS256"],
				}),
			);
		res.writeHead(404).end(
			JSON.stringify({
				error: "Fixture sessions are issued by the prepare command",
			}),
		);
	};
	const server = https.createServer(
		{
			key: readFileSync(`${state}/oidc-tls.pem`),
			cert: readFileSync(`${state}/oidc-cert.pem`),
		},
		handler,
	);
	server.listen(9440, "127.0.0.1", () =>
		console.log(`Local JWKS ready at ${issuer}/jwks`),
	);
	http
		.createServer((req, res) => {
			if (req.url === "/health") return res.writeHead(200).end("ok");
			if (req.url === "/userinfo") return handler(req, res);
			res.writeHead(404).end();
		})
		.listen(9441, "127.0.0.1", () =>
			console.log(`Local UserInfo ready at ${userInfoOrigin}/userinfo`),
		);
} else throw new Error("Use prepare or serve [STATE_DIRECTORY]");
