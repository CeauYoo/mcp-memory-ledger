#!/usr/bin/env bash

set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
project_root="$(cd "${script_dir}/.." && pwd)"
requested_output_dir="${1:-${project_root}/target/reports/self-revision-demo/$(date +%Y%m%d-%H%M%S)}"

cd "${project_root}"

cargo build --bins
build_dir="${CARGO_TARGET_DIR:-${project_root}/target}"
"${build_dir}/debug/run_self_revision_demo" \
  --output-dir "${requested_output_dir}" \
  --server-bin "${build_dir}/debug/agent_llm_mm"

printf 'self-revision demo artifacts: %s\n' "${requested_output_dir}"
