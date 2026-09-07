#!/usr/bin/env bash
# Write a sha256sum-compatible checksum line using tools available on the
# runner.  macOS ships shasum rather than GNU sha256sum.
set -euo pipefail

if [ "$#" -ne 2 ]; then
  echo "usage: $0 FILE OUTPUT" >&2
  exit 2
fi

input=$1
output=$2

if command -v sha256sum >/dev/null 2>&1; then
  sha256sum "$input" > "$output"
elif command -v shasum >/dev/null 2>&1; then
  shasum -a 256 "$input" > "$output"
elif command -v openssl >/dev/null 2>&1; then
  digest=$(openssl dgst -sha256 "$input" | sed 's/^.*= //')
  printf '%s  %s\n' "$digest" "$input" > "$output"
else
  echo "::error::no SHA-256 utility found on runner" >&2
  exit 1
fi
