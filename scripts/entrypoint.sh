#!/bin/sh
# HiveBox container entrypoint.
# Installs extra packages (if requested) before starting the daemon.

set -e

# Install extra Alpine packages if HIVEBOX_PACKAGES is set.
if [ -n "${HIVEBOX_PACKAGES:-}" ]; then
    echo "[hivebox] Installing Alpine packages: $HIVEBOX_PACKAGES"
    apk add --no-cache $HIVEBOX_PACKAGES
fi

# Install extra pip packages if HIVEBOX_PIP_PACKAGES is set.
if [ -n "${HIVEBOX_PIP_PACKAGES:-}" ]; then
    echo "[hivebox] Installing pip packages: $HIVEBOX_PIP_PACKAGES"
    pip install --no-cache-dir --break-system-packages $HIVEBOX_PIP_PACKAGES
fi

# Install extra npm global packages if HIVEBOX_NPM_PACKAGES is set.
if [ -n "${HIVEBOX_NPM_PACKAGES:-}" ]; then
    echo "[hivebox] Installing npm packages: $HIVEBOX_NPM_PACKAGES"
    npm install -g $HIVEBOX_NPM_PACKAGES
fi

# Hand off to hivebox binary with whatever args were passed.
exec hivebox "$@"
