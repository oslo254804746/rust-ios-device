#!/usr/bin/env bash
set -euo pipefail

CRATES=(
  ios-proto
  ios-mux
  ios-lockdown
  ios-tunnel
  ios-xpc
  ios-services
  ios-core
  ios-cli
)

PUBLISH_DELAY_SECONDS="${PUBLISH_DELAY_SECONDS:-90}"
MAX_ATTEMPTS="${MAX_ATTEMPTS:-6}"

publish_one() {
  local crate="$1"
  local attempt=1
  local output
  local status
  local wait_until
  local wait_seconds

  while (( attempt <= MAX_ATTEMPTS )); do
    echo "Publishing ${crate} (attempt ${attempt}/${MAX_ATTEMPTS})"
    output="$(mktemp)"
    set +e
    cargo publish -p "${crate}" --no-verify 2>&1 | tee "${output}"
    status=${PIPESTATUS[0]}
    set -e

    if (( status == 0 )); then
      rm -f "${output}"
      echo "Published ${crate}; waiting ${PUBLISH_DELAY_SECONDS}s for crates.io index/rate limits"
      sleep "${PUBLISH_DELAY_SECONDS}"
      return 0
    fi

    if grep -qiE 'already uploaded|crate version .* is already uploaded|crate .* already exists' "${output}"; then
      rm -f "${output}"
      echo "${crate} is already published; continuing"
      sleep 10
      return 0
    fi

    if grep -qi 'Too Many Requests' "${output}"; then
      wait_until="$(sed -nE 's/.*try again after ([A-Za-z]{3}, [0-9]{2} [A-Za-z]{3} [0-9]{4} [0-9]{2}:[0-9]{2}:[0-9]{2} GMT).*/\1/p' "${output}" | tail -n 1)"
      if [[ -n "${wait_until}" ]]; then
        wait_seconds=$(( $(date -u -d "${wait_until}" +%s) - $(date -u +%s) + 15 ))
        if (( wait_seconds < PUBLISH_DELAY_SECONDS )); then
          wait_seconds="${PUBLISH_DELAY_SECONDS}"
        fi
      else
        wait_seconds=$(( PUBLISH_DELAY_SECONDS * attempt ))
      fi
      rm -f "${output}"
      echo "crates.io rate limited ${crate}; waiting ${wait_seconds}s before retry"
      sleep "${wait_seconds}"
      attempt=$(( attempt + 1 ))
      continue
    fi

    if grep -qiE 'no matching package named `ios-|failed to select a version for the requirement `ios-' "${output}"; then
      wait_seconds=$(( PUBLISH_DELAY_SECONDS * attempt ))
      rm -f "${output}"
      echo "crates.io index has not caught up for ${crate}; waiting ${wait_seconds}s before retry"
      sleep "${wait_seconds}"
      attempt=$(( attempt + 1 ))
      continue
    fi

    cat "${output}" >&2
    rm -f "${output}"
    return "${status}"
  done

  echo "Failed to publish ${crate} after ${MAX_ATTEMPTS} attempts" >&2
  return 1
}

for crate in "${CRATES[@]}"; do
  publish_one "${crate}"
done
