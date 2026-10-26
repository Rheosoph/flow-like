"""Keep code comparisons strict while admitting an explicit storage-layout experiment."""

import compare


VARIANTS = ("baseline", "candidate")


def is_sha256(value):
    return isinstance(value, str) and len(value) == 64 and all(char in "0123456789abcdef" for char in value)


def identity_issues(suite):
    experiment = suite.get("fixtureLayoutExperiment")
    if experiment is None:
        return []
    if not isinstance(experiment, dict):
        return ["Invalid fixture layout experiment declaration"]
    issues = []
    states, fixtures = experiment.get("states"), experiment.get("fixtures")
    if (not isinstance(states, dict) or set(states) != set(VARIANTS)
            or any(not isinstance(value, str) or not value for value in states.values())
            or states.get("baseline") == states.get("candidate")):
        issues.append("Layout experiments require two distinct fixture states")
    if not isinstance(fixtures, dict) or set(fixtures) != set(VARIANTS):
        return [*issues, "Layout experiments require both frozen fixture identities"]
    binaries, frontends = suite.get("binaries"), suite.get("frontends")
    binaries = binaries if isinstance(binaries, dict) else {}
    frontends = frontends if isinstance(frontends, dict) else {}
    hashes = [compare.nested(binaries, variant + ".sha256") for variant in VARIANTS]
    if not all(is_sha256(value) for value in hashes) or hashes[0] != hashes[1]:
        issues.append("Layout experiments require the same backend binary SHA256")
    builds = [frontends.get(variant) for variant in VARIANTS]
    if (any(not isinstance(build, dict) or not is_sha256(build.get("sourceSha256"))
            or not isinstance(build.get("buildId"), str) or not build["buildId"] for build in builds)
            or builds[0] != builds[1]):
        issues.append("Layout experiments require the same production frontend identity")
    for variant in VARIANTS:
        fixture = fixtures[variant]
        if not isinstance(fixture, dict) or not isinstance(fixture.get("database"), dict):
            issues.append(f"{variant}: missing database fixture identity")
            continue
        for field in ("records_sha256", "schema_sha256"):
            if not is_sha256(fixture["database"].get(field)):
                issues.append(f"{variant}: missing valid {field}")
    for field in ("app_id", "graph_source_sha256", "graph_fixture_sha256",
                  "database.records_sha256", "database.schema_sha256", "database.profile",
                  "database.rows", "database.data_storage_version", "database.pylance"):
        a, b = (compare.nested(fixtures[variant], field) for variant in VARIANTS)
        if a is compare.MISSING or b is compare.MISSING or a != b:
            issues.append(f"Layout experiment changed logical fixture identity: {field}")
    return issues


def trial_issues(suite, variant, fixture, state):
    experiment = suite.get("fixtureLayoutExperiment")
    if experiment is None:
        return []
    if not isinstance(experiment, dict):
        return ["Invalid fixture layout experiment declaration"]
    fixtures, states = experiment.get("fixtures"), experiment.get("states")
    if not isinstance(fixtures, dict) or not isinstance(states, dict):
        return ["Incomplete frozen layout experiment identities"]
    issues = []
    if fixture != fixtures.get(variant):
        issues.append("Trial fixture differs from the selected frozen layout")
    if state != states.get(variant):
        issues.append("Trial fixture state differs from the selected variant")
    return issues


def pair_mismatches(first, second, *, layout_experiment=False):
    issues = compare.mismatches(first, second)
    if not layout_experiment:
        a, b = (compare.nested(bundle, "manifest.fixture.database") for bundle in (first, second))
        if a is compare.MISSING or b is compare.MISSING or a != b:
            issues.append("Trials changed physical storage layout without a layout experiment")
    else:
        for field in ("manifest.binary_sha256", "report.web.build.source.sha256", "report.web.build.buildId"):
            a, b = (compare.nested(bundle, field) for bundle in (first, second))
            if a is compare.MISSING or b is compare.MISSING or a != b:
                issues.append(f"Layout experiment changed implementation identity: {field}")
    return issues
