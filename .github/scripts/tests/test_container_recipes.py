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


if __name__ == "__main__":
    unittest.main()
