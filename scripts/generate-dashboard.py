#!/usr/bin/env python3
"""Generate the deterministic six-target support dashboard."""

from __future__ import annotations

import hashlib
import html
import json
import pathlib
import subprocess


ROOT = pathlib.Path(__file__).resolve().parent.parent
QUALIFICATION = ROOT / "qualification"


def load(path: pathlib.Path):
    with path.open("r", encoding="utf-8") as source:
        return json.load(source)


def digest(path: pathlib.Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def require_pass(path: pathlib.Path):
    value = load(path)
    nested_results = []
    for collection in ("proofs", "supported_hosts"):
        nested_results.extend(item.get("result") for item in value.get(collection, []))
    passed = value.get("result") == "pass" or (
        bool(nested_results) and all(result == "pass" for result in nested_results)
    )
    if not passed:
        raise SystemExit(f"qualification is not passing: {path}")
    return value


def escape_list(values: list[str]) -> str:
    return "".join(f"<li>{html.escape(value)}</li>" for value in values)


def main() -> None:
    remu = ROOT / "target" / "debug" / "remu"
    all_manifests = json.loads(
        subprocess.check_output([str(remu), "targets", "--json"], cwd=ROOT)
    )

    spec_path = QUALIFICATION / "dashboard-spec.json"
    spec = load(spec_path)
    baseline_ids = set(spec["targets"])
    manifests = [
        manifest for manifest in all_manifests if manifest["id"] in baseline_ids
    ]
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
    ]
    for path in evidence_paths:
        require_pass(path)

    vendor = load(QUALIFICATION / "vendor-samples.json")
    targets = []
    for manifest in manifests:
        target_id = manifest["id"]
        coverage_path = QUALIFICATION / "register-coverage" / f"{target_id}.json"
        coverage = load(coverage_path)
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
        entry_spec = spec["targets"][target_id]
        targets.append(
            {
                "id": target_id,
                "name": manifest["name"],
                "support_tier": entry_spec["support_tier"],
                "fidelity": manifest["fidelity"],
                "cpu_profiles": manifest["cpus"],
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

    dashboard = {
        "schema": "remu.support-dashboard.v1",
        "portfolio": "six-chip baseline",
        "result": "pass",
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
    json_path.write_text(
        json.dumps(dashboard, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )

    rows = []
    for target in targets:
        profiles = ", ".join(profile["name"] for profile in target["cpu_profiles"])
        coverage = target["register_coverage"]
        rows.append(
            f"""
            <article class="target">
              <header><div><code>{html.escape(target['id'])}</code><h2>{html.escape(target['name'])}</h2></div><span>PROVEN</span></header>
              <p class="tier">{html.escape(target['support_tier'])}</p>
              <dl><dt>CPU profiles</dt><dd>{html.escape(profiles)}</dd>
                <dt>Register evidence</dt><dd>{coverage['covered_register_count']} registers across {len(coverage['required_covered_regions'])} required regions · <a href="register-coverage/{html.escape(target['id'])}.json">manifest</a></dd></dl>
              <div class="columns"><section><h3>Passing corpus</h3><ul>{escape_list(target['passing_corpus'])}</ul></section>
              <section><h3>Known gaps</h3><ul>{escape_list(target['known_gaps'])}</ul></section></div>
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
<section class="provenance"><h2>Provenance and licences</h2><p>Renvo Emulator: MIT OR Apache-2.0. Upstream samples are downloaded at pinned commits, verified by SHA-256, and compiled without changes; tracked adapters provide the SDK and native-MMIO boundary.</p><ul>{sources}</ul><p><a href="dashboard.json">Machine-readable dashboard</a> · <a href="vendor-samples.json">Vendor qualification</a> · <a href="../PLAN.html">Original plan</a></p></section>
</main></body></html>"""
    (QUALIFICATION / "dashboard.html").write_text(page, encoding="utf-8")
    print("generated six-target dashboard: qualification/dashboard.html")


if __name__ == "__main__":
    main()
