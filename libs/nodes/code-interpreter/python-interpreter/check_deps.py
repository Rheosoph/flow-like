#!/usr/bin/env python3
"""Temporary script to check stdlib C extension dependencies."""
import sysconfig
from pathlib import Path

stdlib = Path(sysconfig.get_paths()["stdlib"])

def check_c_deps(mod_path):
    if mod_path.is_file():
        content = mod_path.read_text()
        c_imports = []
        has_fallback = False
        for line in content.splitlines():
            s = line.strip()
            if s.startswith("#"):
                continue
            if "import _" in s and "importlib" not in s:
                c_imports.append(s)
            if "except ImportError" in s:
                has_fallback = True
        return c_imports, has_fallback
    return [], False

modules = [
    "datetime", "calendar", "decimal", "fractions", "numbers", "statistics",
    "string", "difflib", "pprint", "shlex",
    "csv", "configparser", "html", "xml", "email", "mimetypes",
    "http", "urllib", "socket",
    "hashlib", "hmac", "secrets",
    "heapq", "argparse", "logging",
    "locale", "platform", "pickle", "uuid", "tempfile", "signal", "gzip",
    "unittest", "subprocess",
]

for mod in modules:
    py = stdlib / f"{mod}.py"
    pkg = stdlib / mod
    if py.exists():
        c_deps, has_fb = check_c_deps(py)
        status = "OK" if not c_deps else ("FALLBACK" if has_fb else "NEEDS_C")
        deps_str = "; ".join(c_deps[:3]) if c_deps else ""
        print(f"{mod:20s} {status:10s} {deps_str}")
    elif pkg.is_dir():
        init = pkg / "__init__.py"
        if init.exists():
            c_deps, has_fb = check_c_deps(init)
            status = "OK(pkg)" if not c_deps else ("FALLBACK" if has_fb else "NEEDS_C")
            deps_str = "; ".join(c_deps[:3]) if c_deps else ""
            print(f"{mod:20s} {status:10s} {deps_str}")
        else:
            print(f"{mod:20s} PKG_NOINIT")
    else:
        print(f"{mod:20s} NOT_FOUND")
