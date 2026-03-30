#!/usr/bin/env python3
"""Check internal dependencies of fallback modules."""
import ast
import sysconfig
from pathlib import Path

stdlib = Path(sysconfig.get_paths()["stdlib"])

# Key files to analyze
files_to_check = [
    ("_pydatetime.py", "_pydatetime"),
    ("_pydecimal.py", "_pydecimal"),
    ("_strptime.py", "_strptime"),
    ("_compat_pickle.py", "_compat_pickle"),
    ("string.py", "string"),
    ("gzip.py", "gzip"),
    ("tempfile.py", "tempfile"),
    ("heapq.py", "heapq"),
    ("hashlib.py", "hashlib"),
    ("pickle.py", "pickle"),
]

for fname, label in files_to_check:
    f = stdlib / fname
    if not f.exists():
        print(f"\n{label}: NOT FOUND")
        continue
    print(f"\n{label} ({f.stat().st_size} bytes):")
    tree = ast.parse(f.read_text())
    imports = set()
    for node in ast.walk(tree):
        if isinstance(node, ast.Import):
            for alias in node.names:
                imports.add(alias.name.split(".")[0])
        elif isinstance(node, ast.ImportFrom):
            if node.module:
                imports.add(node.module.split(".")[0])
    # Filter to non-builtin, non-self imports
    stdlib_deps = sorted(i for i in imports if i != label and not i.startswith("__"))
    print(f"  deps: {', '.join(stdlib_deps)}")
