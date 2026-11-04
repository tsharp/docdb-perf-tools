#!/usr/bin/env bash

set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repository_root="$(cd -- "${script_dir}/.." && pwd)"
results_dir="${1:-${repository_root}/bench-results}"
output_dir="${2:-${results_dir}/reports}"
timestamp="$(date '+%Y%m%d_%H%M%S')"

mkdir -p -- "${output_dir}"

python3 "${script_dir}/generate_sweep_report.py" \
  --results-dir "${results_dir}" \
  --output "${output_dir}/sweep_report_${timestamp}.html"

python3 "${script_dir}/generate_user_sweep_report.py" \
  --results-dir "${results_dir}" \
  --output "${output_dir}/user_sweep_report_${timestamp}.html"

printf 'Generated reports:\n'
printf '  %s\n' "${output_dir}/sweep_report_${timestamp}.html"
printf '  %s\n' "${output_dir}/user_sweep_report_${timestamp}.html"
