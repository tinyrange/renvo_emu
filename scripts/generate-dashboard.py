#!/usr/bin/env python3
"""Generate the deterministic six-target support dashboard."""

from __future__ import annotations

import hashlib
import html
import json
import os
import pathlib
import subprocess
import sys


ROOT = pathlib.Path(__file__).resolve().parent.parent
QUALIFICATION = ROOT / "qualification"
SUPPORT_TIER_ORDER = {
    "compiler-execution": 0,
    "firmware-functional-slice": 1,
    "board-or-sdk-workflow": 2,
}


def load(path: pathlib.Path):
    with path.open("r", encoding="utf-8") as source:
        return json.load(source)


def digest(path: pathlib.Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def require_pass(path: pathlib.Path):
    value = load(path)
    if value.get("evidence_status") == "passing":
        return value
    nested_results = []
    for collection in ("proofs", "supported_hosts"):
        nested_results.extend(item.get("result") for item in value.get(collection, []))
    passed = value.get("result") == "pass" or (
        bool(nested_results) and all(result == "pass" for result in nested_results)
    )
    if not passed:
        raise SystemExit(f"qualification is not passing: {path}")
    return value


def capability_input_digest(paths: list[pathlib.Path]) -> str:
    """Hash the declared inputs that determine public capability claims."""
    tree = hashlib.sha256()
    for path in sorted(set(paths)):
        relative = path.relative_to(ROOT)
        tree.update(str(relative).encode("utf-8"))
        tree.update(b"\0")
        tree.update(path.read_bytes())
        tree.update(b"\0")
    return tree.hexdigest()


def emit(path: pathlib.Path, content: str, check: bool) -> None:
    if check:
        if not path.exists() or path.read_text(encoding="utf-8") != content:
            raise SystemExit(f"generated artifact is stale: {path.relative_to(ROOT)}")
        return
    path.write_text(content, encoding="utf-8")


def escape_list(values: list[str]) -> str:
    return "".join(f"<li>{html.escape(value)}</li>" for value in values)


def validate_support_tiers(target_id: str, support_tiers: list[dict]) -> None:
    """Ensure manifest tiers are complete, unique, and ordered by scope."""
    if not support_tiers or any(
        not tier.get("name") or not tier.get("evidence") for tier in support_tiers
    ):
        raise SystemExit(f"support tier metadata is incomplete: {target_id}")
    names = [tier["name"] for tier in support_tiers]
    if len(names) != len(set(names)):
        raise SystemExit(f"support tier metadata has duplicate names: {target_id}")
    try:
        ranks = [SUPPORT_TIER_ORDER[name] for name in names]
    except KeyError as error:
        raise SystemExit(
            f"support tier metadata has unknown name {error.args[0]!r}: {target_id}"
        ) from error
    if ranks != sorted(ranks):
        raise SystemExit(f"support tier metadata is out of order: {target_id}")


def resolve_tier_artifact(target_id: str, evidence: str) -> pathlib.Path:
    """Resolve a manifest evidence entry to its target-specific artifact."""
    if evidence.endswith("/"):
        return QUALIFICATION / evidence / f"{target_id}.json"
    return QUALIFICATION / evidence


def main() -> None:
    check = "--check" in sys.argv[1:]
    remu = pathlib.Path(os.environ.get("REMU_BIN", "target/debug/remu"))
    if not remu.is_absolute():
        remu = ROOT / remu
    all_manifests = json.loads(
        subprocess.check_output([str(remu), "targets", "--json"], cwd=ROOT)
    )

    spec_path = QUALIFICATION / "dashboard-spec.json"
    spec = load(spec_path)
    baseline_ids = set(spec["targets"])
    manifests = [
        manifest for manifest in all_manifests if manifest["id"] in baseline_ids
    ]
    if spec.get("schema") != "remu.dashboard-spec.v2":
        raise SystemExit("dashboard spec must use remu.dashboard-spec.v2")
    tier_definitions = spec.get("tier_definitions", [])
    tier_ids = [tier.get("id") for tier in tier_definitions]
    required_tiers = list(SUPPORT_TIER_ORDER)
    if tier_ids != required_tiers:
        raise SystemExit("dashboard tier definitions must be ordered and complete")
    if len(baseline_ids) != 6 or len(manifests) != 6:
        raise SystemExit("dashboard requires all six original target manifests")
    if baseline_ids != {manifest["id"] for manifest in manifests}:
        raise SystemExit("dashboard spec and original target manifests differ")

    evidence_paths = [
        QUALIFICATION / "riscv-cpu.json",
        QUALIFICATION / "arm-cpu.json",
        QUALIFICATION / "xtensa-cpu.json",
        QUALIFICATION / "reduction.json",
        QUALIFICATION / "debug-observability.json",
        QUALIFICATION / "vendor-samples.json",
        QUALIFICATION / "starlark.json",
        QUALIFICATION / "rust-abi.json",
        QUALIFICATION / "stop-conditions.json",
        QUALIFICATION / "host-determinism.json",
        QUALIFICATION / "native-images.json",
    ]
    for path in evidence_paths:
        require_pass(path)

    vendor = load(QUALIFICATION / "vendor-samples.json")
    native_images = load(QUALIFICATION / "native-images.json")
    capability_inputs = [
        pathlib.Path(__file__),
        spec_path,
        ROOT / "crates" / "remu-machines" / "src" / "target.rs",
        QUALIFICATION / "acceptance-report.html",
        *evidence_paths,
    ]
    targets = []
    for manifest in manifests:
        target_id = manifest["id"]
        coverage_path = QUALIFICATION / "register-coverage" / f"{target_id}.json"
        coverage = load(coverage_path)
        capability_inputs.append(coverage_path)
        if coverage.get("evidence_status") != "passing":
            raise SystemExit(f"register evidence is not passing: {target_id}")
        target_vendor = [
            item
            for item in vendor["targets"]
            if item["target"] == target_id
            or item["target"].startswith(f"{target_id}-")
        ]
        if not target_vendor or any(item["result"] != "pass" for item in target_vendor):
            raise SystemExit(f"vendor sample evidence is incomplete: {target_id}")
        entry_spec = spec["targets"].get(target_id)
        if entry_spec is None:
            raise SystemExit(f"dashboard target metadata is missing: {target_id}")
        support_tiers = manifest.get("support_tiers", [])
        validate_support_tiers(target_id, support_tiers)
        if support_tiers[-1]["name"] != entry_spec["highest_tier"]:
            raise SystemExit(f"target tier metadata disagrees: {target_id}")
        all_native_cases = [
            case
            for case in native_images["cases"]
            if case["target"] == target_id
        ]
        if not all_native_cases or any(
            case.get("status") != "pass" for case in all_native_cases
        ):
            raise SystemExit(f"native-image evidence is incomplete: {target_id}")
        native_cases = all_native_cases
        declared_formats = set(entry_spec["native_image_formats"])
        observed_formats = {case["native_format"] for case in native_cases}
        if "elf" not in declared_formats or observed_formats != declared_formats - {"elf"}:
            raise SystemExit(f"native-image formats are not evidence-bound: {target_id}")
        if any(case.get("direct_format") != "elf" for case in native_cases):
            raise SystemExit(f"native-image direct format is not ELF: {target_id}")

        tiers = []
        tier_definitions_by_id = {tier["id"]: tier for tier in tier_definitions}
        for manifest_tier in support_tiers:
            tier_id = manifest_tier["name"]
            tier = tier_definitions_by_id[tier_id]
            artifacts = []
            for evidence in manifest_tier["evidence"]:
                artifact_path = resolve_tier_artifact(target_id, evidence)
                if not artifact_path.exists():
                    raise SystemExit(
                        f"tier artifact is missing: {artifact_path.relative_to(ROOT)}"
                    )
                if artifact_path.suffix == ".json":
                    require_pass(artifact_path)
                artifacts.append(
                    {
                        "path": str(artifact_path.relative_to(ROOT)),
                        "sha256": digest(artifact_path),
                    }
                )
            if tier_id == "compiler-execution":
                omissions = [
                    "chip-specific peripheral and board behavior is not implied by compiler execution"
                ]
            elif tier_id == "firmware-functional-slice":
                omissions = list(manifest["limitations"])
            else:
                omissions = [
                    "only the named workflow is qualified; arbitrary SDK or production firmware is not implied"
                ]
            tiers.append(
                {
                    "id": tier_id,
                    "label": tier["label"],
                    "description": tier["description"],
                    "status": "proven",
                    "evidence": artifacts,
                    "known_omissions": omissions,
                }
            )

        cpu_rows = [
            {
                "id": case["id"],
                "native_image_format": case["native_format"],
                "status": case["status"],
                "native_result_sha256": case.get("native_sha256"),
            }
            for case in native_cases
        ]
        targets.append(
            {
                "id": target_id,
                "name": manifest["name"],
                "highest_tier": entry_spec["highest_tier"],
                "support_tiers": tiers,
                "fidelity": manifest["fidelity"],
                "cpu_profiles": manifest["cpus"],
                "cpu_evidence_rows": cpu_rows,
                "native_image_formats": entry_spec["native_image_formats"],
                "peripheral_scope": entry_spec["peripheral_scope"],
                "official_workflows": entry_spec["official_workflows"],
                "peripheral_tracker": entry_spec["peripheral_tracker"],
                "passing_corpus": entry_spec["passing_corpus"],
                "register_coverage": {
                    "status": coverage["evidence_status"],
                    "covered_register_count": coverage["covered_register_count"],
                    "required_covered_regions": coverage["required_covered_regions"],
                    "manifest": str(coverage_path.relative_to(ROOT)),
                    "manifest_sha256": digest(coverage_path),
                },
                "known_gaps": manifest["limitations"] + coverage["known_deviations"],
                "documentation_sources": manifest["sources"],
                "vendor_sample_profiles": [item["target"] for item in target_vendor],
            }
        )

    input_digest = capability_input_digest(capability_inputs)
    dashboard = {
        "schema": "remu.support-dashboard.v2",
        "portfolio": "six-chip baseline",
        "result": "pass",
        "capability_input_sha256": input_digest,
        "tier_definitions": tier_definitions,
        "scope_note": (
            "Baseline proven means deterministic functional compiler/firmware testing; "
            "it does not claim cycle accuracy, complete ISA coverage, or complete peripherals."
        ),
        "targets": targets,
        "provenance_and_licences": {
            "remu": "MIT OR Apache-2.0",
            "vendor_samples": vendor["sources"],
            "adapter_boundary": vendor["adapter_boundary"],
        },
        "phase_5_evidence": [
            {
                "path": str(path.relative_to(ROOT)),
                "sha256": digest(path),
            }
            for path in evidence_paths
        ],
        "generator": {
            "path": "scripts/generate-dashboard.py",
            "sha256": digest(pathlib.Path(__file__)),
            "spec": str(spec_path.relative_to(ROOT)),
            "spec_sha256": digest(spec_path),
        },
    }

    json_path = QUALIFICATION / "dashboard.json"
    dashboard_json = json.dumps(dashboard, indent=2, sort_keys=True) + "\n"

    rows = []
    for target in targets:
        profiles = ", ".join(profile["name"] for profile in target["cpu_profiles"])
        coverage = target["register_coverage"]
        cpu_rows = escape_list(
            [
                f"{row['id']} · {row['native_image_format']} · {row['status']}"
                for row in target["cpu_evidence_rows"]
            ]
        )
        tiers = "".join(
            f"<li><strong>{html.escape(tier['label'])}</strong> — {html.escape(tier['status'])}; "
            f"evidence: {escape_list([item['path'] + ' (' + item['sha256'][:12] + ')' for item in tier['evidence']])}"
            f"</li>"
            for tier in target["support_tiers"]
        )
        rows.append(
            f"""
            <article class="target">
              <header><div><code>{html.escape(target['id'])}</code><h2>{html.escape(target['name'])}</h2></div><span>{html.escape(target['support_tiers'][-1]['label'].upper())}</span></header>
              <p class="tier">Highest tier: {html.escape(target['support_tiers'][-1]['label'])}. This is a named evidence claim, not arbitrary-device compatibility.</p>
              <dl><dt>CPU profiles</dt><dd>{html.escape(profiles)}</dd>
                <dt>Native-image rows</dt><dd><ul>{cpu_rows}</ul></dd>
                <dt>Native formats</dt><dd>{html.escape(', '.join(target['native_image_formats']))}</dd>
                <dt>Peripheral scope</dt><dd>{html.escape(', '.join(target['peripheral_scope']))} · <a href="{html.escape(target['peripheral_tracker'])}">tracker</a></dd>
                <dt>Register evidence</dt><dd>{coverage['covered_register_count']} registers across {len(coverage['required_covered_regions'])} required regions · <a href="register-coverage/{html.escape(target['id'])}.json">manifest</a></dd></dl>
              <div class="columns"><section><h3>Support tiers</h3><ul>{tiers}</ul></section>
              <section><h3>Official workflows</h3><ul>{escape_list(target['official_workflows'])}</ul><h3>Passing corpus</h3><ul>{escape_list(target['passing_corpus'])}</ul></section></div>
              <p><strong>Known gaps:</strong> {html.escape('; '.join(target['known_gaps']))}</p>
            </article>"""
        )

    sources = "".join(
        f"<li><a href=\"{html.escape(source['repository'])}\">{html.escape(source['corpus'])}</a> · <code>{html.escape(source['commit'][:12])}</code> · {html.escape(source['licence'])}</li>"
        for source in vendor["sources"]
    )
    page = f"""<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>Renvo Emulator six-target support dashboard</title><style>
:root{{--bg:#0b0e0d;--panel:#131816;--line:#29342f;--text:#e8efeb;--muted:#9baba3;--green:#73e2ae}}*{{box-sizing:border-box}}body{{margin:0;background:var(--bg);color:var(--text);font:15px/1.55 system-ui,sans-serif}}main{{width:min(1100px,calc(100% - 32px));margin:0 auto;padding:64px 0 96px}}h1{{font-size:clamp(38px,7vw,72px);line-height:1;margin:.15em 0}}.lede,.tier,dd{{color:var(--muted)}}.notice,.target,.provenance{{border:1px solid var(--line);border-radius:14px;background:var(--panel);padding:24px;margin:18px 0}}.notice{{border-left:3px solid var(--green)}}header{{display:flex;justify-content:space-between;gap:20px;align-items:start}}header span{{color:var(--green);font:700 12px monospace;letter-spacing:.12em}}h2{{margin:.2em 0}}h3{{font-size:14px}}code{{color:var(--green)}}dl{{display:grid;grid-template-columns:150px 1fr;gap:7px 16px}}dt{{font-weight:700}}dd{{margin:0}}.columns{{display:grid;grid-template-columns:1fr 1fr;gap:28px}}a{{color:var(--green)}}li{{margin:.35em 0}}@media(max-width:700px){{.columns{{grid-template-columns:1fr}}dl{{grid-template-columns:1fr}}}}
</style></head><body><main><p><code>REMU / QUALIFICATION</code></p><h1>Six-target support dashboard</h1>
<p class="lede">Checked compiler, firmware, register, and observability evidence for the original baseline.</p>
<div class="notice"><strong>What “proven” means.</strong> {html.escape(dashboard['scope_note'])}</div>
{''.join(rows)}
<section class="provenance"><h2>Provenance and licences</h2><p>Renvo Emulator: MIT OR Apache-2.0. Upstream samples are downloaded at pinned commits, verified by SHA-256, and compiled without changes; tracked adapters provide the SDK and native-MMIO boundary.</p><ul>{sources}</ul><p>Capability input digest: <code>{input_digest}</code></p><p><a href="dashboard.json">Machine-readable dashboard</a> · <a href="capability-matrix.md">Markdown capability matrix</a> · <a href="vendor-samples.json">Vendor qualification</a> · <a href="../PLAN.html">Original plan</a></p></section>
</main></body></html>"""
    matrix_rows = [
        "# Renvo Emulator capability matrix",
        "",
        "This matrix is generated from target manifests and checked qualification artifacts. Tier 3 is a named workflow claim, not arbitrary SDK or production-firmware compatibility.",
        "",
        f"Capability input SHA-256: `{input_digest}`",
        "",
        "| Target | Highest tier | CPU evidence rows | Native formats | Peripheral scope | Official workflow | Tracker |",
        "| --- | --- | --- | --- | --- | --- | --- |",
    ]
    for target in targets:
        cpu_rows = "; ".join(
            f"{row['id']} ({row['native_image_format']})" for row in target["cpu_evidence_rows"]
        )
        workflows = "; ".join(target["official_workflows"])
        matrix_rows.append(
            "| "
            + " | ".join(
                [
                    f"[{target['name']}]({target['peripheral_tracker']})",
                    target["support_tiers"][-1]["label"],
                    cpu_rows,
                    ", ".join(target["native_image_formats"]),
                    ", ".join(target["peripheral_scope"]),
                    workflows,
                    target["peripheral_tracker"],
                ]
            )
            + " |"
        )
    matrix_rows.extend(
        [
            "",
            "## Tier definitions",
            "",
            *[
                f"- **{tier['label']}** — {tier['description']}"
                for tier in tier_definitions
            ],
            "",
            "Every tier above is bound to artifact paths and SHA-256 digests in `dashboard.json`; `scripts/check-capability-matrix.sh` rejects stale generated outputs.",
            "",
        ]
    )
    matrix = "\n".join(matrix_rows)
    emit(json_path, dashboard_json, check)
    emit(QUALIFICATION / "dashboard.html", page, check)
    emit(QUALIFICATION / "capability-matrix.md", matrix, check)
    print("checked" if check else "generated", "capability matrix and six-target dashboard")


if __name__ == "__main__":
    main()
