package buildcontext

import (
	"os"
	"path/filepath"
	"testing"

	"github.com/moby/patternmatcher"
	"github.com/moby/patternmatcher/ignorefile"
)

func contextMatcher(t *testing.T) (string, *patternmatcher.PatternMatcher) {
	t.Helper()
	root, err := os.Getwd()
	if err != nil {
		t.Fatal(err)
	}
	for {
		if _, err := os.Stat(filepath.Join(root, "Cargo.toml")); err == nil {
			break
		}
		parent := filepath.Dir(root)
		if parent == root {
			t.Fatal("repository root not found")
		}
		root = parent
	}
	file, err := os.Open(filepath.Join(root, ".dockerignore"))
	if err != nil {
		t.Fatal(err)
	}
	defer file.Close()
	patterns, err := ignorefile.ReadAll(file)
	if err != nil {
		t.Fatal(err)
	}
	matcher, err := patternmatcher.New(patterns)
	if err != nil {
		t.Fatal(err)
	}
	return root, matcher
}

func TestRequiredWorkspaceInputsRemainInContext(t *testing.T) {
	root, matcher := contextMatcher(t)
	paths := []string{
		"Cargo.toml", "Cargo.lock", ".cargo/config.toml", "package.json", "bun.lock",
		"packages/api/Cargo.toml", "packages/api/src/lib.rs",
		"packages/secrets/Cargo.toml", "packages/secrets/src/lib.rs",
		"packages/compiler/Cargo.toml", "packages/compiler/src/lib.rs",
		"packages/executor/Cargo.toml", "packages/executor/src/lib.rs",
		"packages/locales/package.json", "packages/ui/package.json", "apps/web/package.json",
		"scripts/sync-node-icons.ts", "apps/backend/docker-compose/api/src/main.rs",
		"apps/backend/docker-compose/runtime/src/main.rs",
		"apps/backend/docker-compose/runtime/src/once.rs",
		"apps/backend/execution-manager/Cargo.toml",
		"apps/backend/execution-manager/src/lib.rs",
		"apps/backend/execution-manager/src/server.rs",
		"apps/backend/execution-manager/src/docker/engine.rs",
		"apps/backend/execution-manager/src/gateway/proxy.rs",
		"apps/backend/shared/api_hardening.rs",
		"apps/backend/kubernetes/api/src/main.rs",
		"apps/backend/execution-manager/src/kubernetes/mod.rs",
		"apps/backend/execution-manager/src/kubernetes/slot.rs",
		"apps/backend/kubernetes/flow-like.config.example.json",
		"apps/backend/docker-compose/flow-like.config.example.json",
	}
	err := filepath.WalkDir(filepath.Join(root, "packages/secrets/src"), func(path string, entry os.DirEntry, err error) error {
		if err != nil {
			return err
		}
		if !entry.IsDir() {
			relative, _ := filepath.Rel(root, path)
			paths = append(paths, filepath.ToSlash(relative))
		}
		return nil
	})
	if err != nil {
		t.Fatal(err)
	}
	for _, path := range paths {
		t.Run(path, func(t *testing.T) {
			if _, err := os.Stat(filepath.Join(root, path)); err != nil {
				t.Fatal(err)
			}
			excluded, err := matcher.MatchesOrParentMatches(path)
			if err != nil {
				t.Fatal(err)
			}
			if excluded {
				t.Fatalf("required source %s is excluded from the Docker build context", path)
			}
		})
	}
}

func TestSyntheticDeploymentSecretsAreExcluded(t *testing.T) {
	_, matcher := contextMatcher(t)
	// Evaluate synthetic paths without opening any operator credential file.
	for _, path := range []string{
		".env", "real.env", "apps/backend/docker-compose/real.env",
		"apps/backend/docker-compose/.env", "packages/api/.env.local",
		"secrets/backend.key", "apps/backend/docker-compose/secrets/issuer.key",
		"apps/backend/docker-compose/.secrets/backend.key", "keys/backend.pem",
		".codex/config.toml", ".agents/private-config.json",
		".local-notes/self-hosting/OBJECT_STORE_RESEARCH.md",
		"apps/backend/kubernetes/.generated/secrets.yaml",
		"apps/backend/kubernetes/.generated/nested/backend.key",
		"apps/backend/kubernetes/helm/values-secrets.yaml",
	} {
		t.Run(path, func(t *testing.T) {
			excluded, err := matcher.MatchesOrParentMatches(path)
			if err != nil {
				t.Fatal(err)
			}
			if !excluded {
				t.Fatalf("credential path %s enters the Docker build context", path)
			}
		})
	}
}
