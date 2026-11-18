"""Require an explicit publication decision for every deployment Dockerfile."""

import importlib.util
from pathlib import Path
import re
import unittest


ROOT = Path(__file__).resolve().parents[3]
SPEC = importlib.util.spec_from_file_location("container_images", ROOT / ".github/scripts/container_images.py")
containers = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(containers)


class RecipeCoverageTests(unittest.TestCase):
    def test_all_default_recipes_are_published(self):
        recipes = {
            str(path.relative_to(ROOT))
            for backend in ("aws", "gcp", "azure", "docker-compose", "kubernetes", "audit-worker")
            for path in (ROOT / "apps/backend" / backend).rglob("Dockerfile")
        }
        published = {entry["dockerfile"] for entry in containers.matrix("all")["include"]}
        self.assertEqual(recipes, published)

    def test_each_published_final_stage_includes_the_repository_license(self):
        for recipe in sorted({entry["dockerfile"] for entry in containers.matrix("all")["include"]}):
            with self.subTest(recipe=recipe):
                stages = re.split(r"(?im)^FROM\s+", (ROOT / recipe).read_text())
                self.assertIn("COPY LICENSE /usr/share/licenses/flow-like/LICENSE", stages[-1])

    def test_non_aws_api_consumers_honor_job_limit_and_server_output_paths(self):
        recipes = {entry["dockerfile"] for entry in containers.matrix("all")["include"]
                   if entry["cloud"] != "aws" and entry["workload"] in {"api", "audit-worker", "runtime"}}
        for recipe in sorted(recipes):
            with self.subTest(recipe=recipe):
                text = (ROOT / recipe).read_text()
                self.assertIn("ARG CARGO_BUILD_JOBS=1\n", text)
                self.assertIn("ENV CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS}\n", text)
                package = re.search(r"cargo build --locked --profile server --package ([\w-]+)", text)
                self.assertIsNotNone(package, "API consumers must use the server profile")
                self.assertIn(f"cp /app/target/server/{package[1]} /tmp/{package[1]}", text)
                self.assertNotIn("/target/release/", text)
                self.assertRegex(text, rf"strip (?:-s|--strip-all) /(?:app/target/server|tmp)/{re.escape(package[1])}\b")

    def test_web_bundle_stages_bound_cargo_and_editor_parallelism(self):
        for cloud in ("docker-compose", "kubernetes"):
            with self.subTest(cloud=cloud):
                text = (ROOT / f"apps/backend/{cloud}/web/Dockerfile").read_text()
                stage = re.search(r"(?ms)^FROM [^\n]+ AS device-crypto\n(.*?)(?=^FROM )", text)[1]
                self.assertIn("ARG CARGO_BUILD_JOBS=2\n", stage)
                self.assertIn("ARG CRAFT_BUILD_CONCURRENCY=1\n", stage)
                # Both controls must reach the Node scripts and their Cargo children.
                before_build = stage.split("node packages/device-crypto/build-web.mjs", 1)[0]
                self.assertIn("CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS}", before_build)
                self.assertIn("CRAFT_BUILD_CONCURRENCY=${CRAFT_BUILD_CONCURRENCY}", before_build)


if __name__ == "__main__":
    unittest.main()
