#!/bin/sh
set -eu
# Same-user approval artifacts are not authorization.  Use a reviewed,
# manifest-only trusted-base transition PR instead.
printf 'self-issued staged approvals are not supported\n' >&2
exit 2
