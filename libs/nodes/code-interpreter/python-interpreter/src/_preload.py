"""
Force componentize-py to bundle stdlib and vendor packages into the WASM component.

componentize-py only bundles modules transitively imported from the entry point.
This module tries importing everything we want available at runtime.
Imports that fail at build time (missing C extensions) are silently skipped;
the module files are still included if they're on the -p path AND all their
transitive imports succeed.
"""

def _try_import(name):
    try:
        __import__(name)
    except (ImportError, ModuleNotFoundError):
        # Missing modules are expected in this environment; failures are ignored
        # so that build-time bundling can proceed best-effort.
        pass

# ── Stdlib: internal fallback modules (must come first) ─────────────────────

_try_import("_pydatetime")
_try_import("_pydecimal")
_try_import("_strptime")
_try_import("_compat_pickle")
_try_import("_compression")
_try_import("copyreg")

# ── Stdlib: public modules ──────────────────────────────────────────────────

_STDLIB = [
    "datetime", "calendar", "decimal", "fractions", "numbers", "statistics",
    "string", "difflib", "pprint", "shlex",
    "csv", "configparser", "html", "xml", "email", "mimetypes",
    "http", "urllib",
    "hashlib", "hmac", "secrets",
    "heapq", "argparse", "logging",
    "gzip", "pickle",
    "locale", "platform", "uuid", "tempfile", "unittest",
]

for _m in _STDLIB:
    _try_import(_m)

# ── Pre-bundled PyPI packages ───────────────────────────────────────────────

_PYPI = [
    # HTTP / networking
    "requests", "urllib3", "charset_normalizer", "certifi", "idna",
    "httpx", "httpcore", "anyio", "sniffio", "h11",
    # HTML / XML / templating
    "bs4", "soupsieve", "jinja2", "markupsafe",
    "defusedxml", "xmltodict",
    # Data serialization / config
    "toml", "tomli", "json5", "jsonlines",
    # Validation / typing
    "pydantic", "annotated_types", "typing_extensions",
    "attrs", "marshmallow", "dataclasses_json", "marshmallow_enum",
    "validators",
    # Date / time / locale
    "dateutil", "dateutil.parser", "dateutil.relativedelta", "dateutil.tz",
    "dateutil.rrule", "dateutil.easter", "dateutil.utils",
    "six", "pytz", "isodate",
    # Text / formatting
    "tabulate", "texttable", "humanize", "slugify", "text_unidecode",
    "colorama", "pygments", "rich", "wcwidth",
    "markdown_it", "mdurl",
    # CLI / utility
    "click", "tqdm", "tenacity", "dotenv", "semver",
    "packaging", "packaging.version", "packaging.specifiers",
    "packaging.requirements", "packaging.markers", "packaging.utils",
    "pyparsing", "chardet", "more_itertools",
    # JSON processing
    "jsonpath_ng", "jsonpath_ng.ext", "jsonpath_ng.jsonpath", "ply",
    # Sub-modules that use lazy loading and need explicit imports
    "marshmallow.fields", "marshmallow.validate", "marshmallow.exceptions",
    "marshmallow.schema", "marshmallow.decorators", "marshmallow.utils",
    "pydantic.fields", "pydantic.main",
    "rich.console", "rich.text", "rich.table",
    "jinja2.environment", "jinja2.loaders",
    "bs4.element", "bs4.builder",
    "requests.api", "requests.sessions", "requests.models",
    "httpx._client", "httpx._api",
    "urllib3.util", "urllib3.util.retry",
]

for _m in _PYPI:
    _try_import(_m)
