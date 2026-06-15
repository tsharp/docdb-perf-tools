#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd -- "${script_dir}/.." && pwd)"

image_name="benchly"
image_tag="local"
image_ref=""
output_dir="${repo_root}/bench-results"
container_output_dir="/app/bench-results"
mongodb_url_file=""
skip_build=false
build_only=false
dry_run=false
benchly_args=()

usage() {
  cat <<'USAGE'
Usage:
  scripts/docker-build-run.sh [options] [benchly options]
  scripts/docker-build-run.sh [options] -- [benchly options]

Builds the local Benchly Docker image and runs benchly in a container.

Options:
  --mongodb-url-file PATH     Host file containing the MongoDB connection string.
                              The file is mounted read-only into the container.
  --output-dir PATH           Host directory for benchmark reports.
                              Default: ./bench-results
  --image NAME[:TAG]          Full Docker image reference. Default: benchly:local
  --image-name NAME           Docker image name. Default: benchly
  --tag TAG                   Docker image tag. Default: local
  --container-output-dir PATH Container path for reports. Default: /app/bench-results
  --skip-build                Run the existing image without rebuilding.
  --build-only                Build the image and exit.
  --dry-run                   Print Docker commands without running them.
  -h, --help                  Show this help.

Examples:
  scripts/docker-build-run.sh --build-only

  scripts/docker-build-run.sh \
    --mongodb-url-file ./local.secret \
    --test write \
    --workers 8 \
    --duration 120 \
    --run-label docker_write_smoke

  BENCHLY_MONGODB_URL='mongodb://...' scripts/docker-build-run.sh \
    --test read \
    --workers 8 \
    --no-drop-collection

Anything not listed above is passed through to benchly.
USAGE
}

fail() {
  printf 'Error: %s\n' "$1" >&2
  exit 1
}

require_value() {
  local option="$1"
  local value="${2:-}"
  if [[ -z "${value}" || "${value}" == --* ]]; then
    fail "${option} requires a value."
  fi
}

require_command() {
  local command_name="$1"
  if ! command -v "${command_name}" >/dev/null 2>&1; then
    fail "${command_name} is not installed or not available in PATH."
  fi
}

resolve_path() {
  local path="$1"
  if command -v realpath >/dev/null 2>&1; then
    realpath -m "$path"
  else
    local path_dir=""
    local path_base=""
    path_dir="$(dirname -- "$path")"
    path_base="$(basename -- "$path")"
    printf '%s/%s\n' "$(cd -- "${path_dir}" && pwd)" "${path_base}"
  fi
}

has_benchly_arg() {
  local option="$1"
  local argument=""
  for argument in "${benchly_args[@]}"; do
    if [[ "${argument}" == "${option}" || "${argument}" == "${option}="* ]]; then
      return 0
    fi
  done
  return 1
}

print_command() {
  local redact_next=false
  local argument=""
  for argument in "$@"; do
    if [[ "${argument}" == --mongodb-url=* ]]; then
      printf ' %q' '--mongodb-url=<mongodb-url>'
      continue
    fi

    if [[ "${redact_next}" == true ]]; then
      printf ' %q' '<mongodb-url>'
      redact_next=false
      continue
    fi

    printf ' %q' "${argument}"
    if [[ "${argument}" == "--mongodb-url" ]]; then
      redact_next=true
    fi
  done
  printf '\n'
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    -h|--help)
      usage
      exit 0
      ;;
    --mongodb-url-file)
      require_value "$1" "${2:-}"
      mongodb_url_file="$2"
      shift 2
      ;;
    --output-dir)
      require_value "$1" "${2:-}"
      output_dir="$2"
      shift 2
      ;;
    --image)
      require_value "$1" "${2:-}"
      image_ref="$2"
      shift 2
      ;;
    --image-name)
      require_value "$1" "${2:-}"
      image_name="$2"
      shift 2
      ;;
    --tag|--image-tag)
      require_value "$1" "${2:-}"
      image_tag="$2"
      shift 2
      ;;
    --container-output-dir)
      require_value "$1" "${2:-}"
      container_output_dir="$2"
      shift 2
      ;;
    --skip-build)
      skip_build=true
      shift
      ;;
    --build-only)
      build_only=true
      shift
      ;;
    --dry-run)
      dry_run=true
      shift
      ;;
    --)
      shift
      benchly_args+=("$@")
      break
      ;;
    *)
      benchly_args+=("$1")
      shift
      ;;
  esac
done

if [[ -z "${image_ref}" ]]; then
  image_ref="${image_name}:${image_tag}"
fi

if [[ "${dry_run}" == false ]]; then
  require_command docker
fi

dockerfile_path="${repo_root}/Dockerfile"
[[ -f "${dockerfile_path}" ]] || fail "Dockerfile not found: ${dockerfile_path}"

build_command=(docker build --tag "${image_ref}" --file "${dockerfile_path}" "${repo_root}")

if [[ "${skip_build}" == false ]]; then
  if [[ "${dry_run}" == true ]]; then
    printf 'Would run:'
    print_command "${build_command[@]}"
  else
    printf 'Building Docker image: %s\n' "${image_ref}"
    "${build_command[@]}"
  fi
fi

if [[ "${build_only}" == true ]]; then
  exit 0
fi

docker_run_args=(docker run --rm --init)

if [[ "${dry_run}" == false ]]; then
  mkdir -p "${output_dir}"
fi
resolved_output_dir="$(resolve_path "${output_dir}")"
docker_run_args+=(--volume "${resolved_output_dir}:${container_output_dir}")

if [[ -n "${mongodb_url_file}" ]]; then
  if [[ "${dry_run}" == false ]]; then
    [[ -f "${mongodb_url_file}" ]] || fail "MongoDB URL file not found: ${mongodb_url_file}"
  fi

  resolved_mongodb_url_file="$(resolve_path "${mongodb_url_file}")"
  container_mongodb_url_file="/tmp/benchly-mongodb-url.secret"
  docker_run_args+=(--volume "${resolved_mongodb_url_file}:${container_mongodb_url_file}:ro")

  if ! has_benchly_arg "--mongodb-url-file" && ! has_benchly_arg "--mongodb-url"; then
    benchly_args=(--mongodb-url-file "${container_mongodb_url_file}" "${benchly_args[@]}")
  fi
fi

if [[ -n "${BENCHLY_MONGODB_URL:-}" ]]; then
  docker_run_args+=(--env BENCHLY_MONGODB_URL)
fi

if [[ -n "${MONGODB_URL:-}" ]]; then
  docker_run_args+=(--env MONGODB_URL)
fi

if ! has_benchly_arg "--output-dir"; then
  benchly_args=(--output-dir "${container_output_dir}" "${benchly_args[@]}")
fi

docker_run_args+=(--env RUST_BACKTRACE=1 "${image_ref}" "${benchly_args[@]}")

if [[ "${dry_run}" == true ]]; then
  printf 'Would run:'
  print_command "${docker_run_args[@]}"
  exit 0
fi

printf 'Running Benchly with Docker image: %s\n' "${image_ref}"
"${docker_run_args[@]}"