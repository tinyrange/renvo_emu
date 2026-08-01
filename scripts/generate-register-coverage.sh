#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$repo_root"

artifact_root=${1:-.remu/portfolio-smoke}
output_root=${2:-qualification/register-coverage}
spec=qualification/register-coverage-spec.json

command -v jq >/dev/null
command -v sha256sum >/dev/null
jq -e '.schema == "remu.register-coverage-spec.v1" and (.targets | length == 6)' "$spec" >/dev/null

work=$(mktemp -d)
trap 'rm -rf -- "$work"' EXIT HUP INT TERM
mkdir -p "$output_root"
spec_sha=$(sha256sum "$spec" | cut -d ' ' -f 1)

for target in $(jq -r '.targets | keys[]' "$spec")
do
    case "$target" in
        ch32v003|ch32v006|rp2040|rp2350|esp32s3|esp32c6) ;;
        *) echo "unsafe target in register coverage spec: $target" >&2; exit 1 ;;
    esac

    config=$(jq -c --arg target "$target" '.targets[$target]' "$spec")
    proofs="$work/$target-proofs.ndjson"
    registers="$work/$target-registers.json"
    : > "$proofs"
    bus_files=

    for proof in $(printf '%s' "$config" | jq -r '.proofs[]')
    do
        case "$proof" in
            *[!a-zA-Z0-9._-]*|'') echo "unsafe proof ID: $proof" >&2; exit 1 ;;
        esac
        result="$artifact_root/$proof-run.json"
        bus="$artifact_root/$proof-bus.json"
        test -f "$result"
        test -f "$bus"
        jq -e '.reason == "Halted" and .exit_code != null' "$result" >/dev/null
        jq -e 'type == "array" and all(.[]; has("region") and has("address") and has("kind") and has("width"))' "$bus" >/dev/null
        result_sha=$(sha256sum "$result" | cut -d ' ' -f 1)
        bus_sha=$(sha256sum "$bus" | cut -d ' ' -f 1)
        jq -n \
            --arg id "$proof" \
            --arg result "$result" \
            --arg bus_log "$bus" \
            --arg result_sha256 "$result_sha" \
            --arg bus_log_sha256 "$bus_sha" \
            --argjson outcome "$(jq '{reason, exit_code, trace_digest, stats}' "$result")" \
            '{id: $id, result: $result, bus_log: $bus_log, result_sha256: $result_sha256, bus_log_sha256: $bus_log_sha256, outcome: $outcome}' \
            >> "$proofs"
        bus_files="$bus_files $bus"
    done

    # Word splitting is intentional: proof IDs are validated above and resolve
    # beneath the caller-supplied artifact directory.
    # shellcheck disable=SC2086
    jq -s '
        [ .[][]
          | select(.kind != "Execute")
          | select(.region != "remu.test.exit")
          | select(.region | test("(^|[.])(flash|ram|sram|dram|iram|rom|irom|xip)([.]|$)"; "i") | not)
        ]
        | sort_by(.region, .address, .kind, .width)
        | group_by([.region, .address])
        | map({
            region: .[0].region,
            address: .[0].address,
            operations: (map(.kind) | unique | sort),
            widths: (map(.width) | unique | sort),
            access_count: length
          })
        | sort_by(.region, .address)
    ' $bus_files > "$registers"

    for region in $(printf '%s' "$config" | jq -r '.required_covered_regions[]')
    do
        jq -e --arg region "$region" 'any(.[]; .region == $region)' "$registers" >/dev/null || {
            echo "$target proof set did not cover required region $region" >&2
            exit 1
        }
    done

    jq -n \
        --arg schema "remu.register-coverage.v1" \
        --arg target "$target" \
        --arg generated_by "scripts/generate-register-coverage.sh" \
        --arg source_spec "$spec" \
        --arg source_spec_sha256 "$spec_sha" \
        --argjson config "$config" \
        --slurpfile proofs "$proofs" \
        --slurpfile registers "$registers" \
        '{
          schema: $schema,
          target: $target,
          cpu_profiles: $config.cpu_profiles,
          evidence_status: "passing",
          generated_by: $generated_by,
          source_spec: {path: $source_spec, sha256: $source_spec_sha256},
          proofs: $proofs,
          required_covered_regions: $config.required_covered_regions,
          covered_register_count: ($registers[0] | length),
          covered_registers: $registers[0],
          additional_evidence: $config.additional_evidence,
          known_deviations: $config.known_deviations
        }' > "$output_root/$target.json"
done

test "$(find "$output_root" -maxdepth 1 -name '*.json' -type f | wc -l)" -eq 6
echo "generated six passing register coverage manifests in $output_root"
