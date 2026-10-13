#!/usr/bin/env bash

set -euo pipefail

usage() {
  cat >&2 <<'USAGE'
usage: ./scripts/provider-live-certification-run.sh (--live | --stub-evidence) [config_path] [evidence_root]

Generates provider certification evidence files. --live calls the configured
provider endpoint (at most two sends, 16 KiB input JSON and 512 output tokens per
request, no retry). Unknown prices prevent a software-enforced dollar cap.
Use a new evidence root; loopback fixtures are offline evidence. --stub-evidence writes
explicit stub/simulated evidence that does not satisfy live certification.

Omitted or conflicting mode is rejected; choose --live or --stub-evidence.
USAGE
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  usage
  exit 0
fi

stub_evidence=""
live_mode=""
mode_count=0
while [[ $# -gt 0 ]]; do
  case "${1}" in
    --stub-evidence)
      stub_evidence="yes"
      mode_count=$((mode_count + 1))
      shift
      ;;
    --live)
      live_mode="yes"
      mode_count=$((mode_count + 1))
      shift
      ;;
    -*)
      usage
      exit 2
      ;;
    *)
      break
      ;;
  esac
done

if [[ "${mode_count}" -ne 1 ]]; then
  usage
  echo "choose exactly one mode: --live or --stub-evidence" >&2
  exit 2
fi

if [[ $# -gt 2 ]]; then
  usage
  exit 2
fi

config_path="${1:-}"
evidence_root="${2:-.}"

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
project_root="$(cd "${script_dir}/.." && pwd -P)"
cd "${project_root}"

args=(--evidence-root "${evidence_root}")
if [[ -n "${stub_evidence}" ]]; then
  args+=(--stub-evidence)
elif [[ -n "${live_mode}" ]]; then
  args+=(--live)
fi
if [[ -n "${config_path}" ]]; then
  args+=(--config-path "${config_path}")
fi

cargo run --quiet --features release-tools --bin provider_live_certification_run -- "${args[@]}"
