#!/bin/sh
set -eu
exec python "$(dirname "$0")/run.py" "$@"
