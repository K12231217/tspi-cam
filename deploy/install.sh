#!/usr/bin/env bash
# Install tspi-cam on the Taishan Pi. Run on the board as root (or with
# sudo) from the cloned repository directory.
#   sudo ./deploy/install.sh
# Never overwrites an existing /etc/tspi-cam/actiond.toml.
set -euo pipefail
cd "$(dirname "$0")/.."

install -d /usr/local/bin
install -m 0755 build/bin/actiond /usr/local/bin/actiond
install -m 0755 build/bin/actionctl /usr/local/bin/actionctl
install -m 0755 build/bin/tspi-cam-ui /usr/local/bin/tspi-cam-ui
install -m 0755 build/bin/actioncal /usr/local/bin/actioncal

install -d /etc/tspi-cam
if [ ! -f /etc/tspi-cam/actiond.toml ]; then
    install -m 0644 config/actiond.toml /etc/tspi-cam/actiond.toml
    echo "installed config template to /etc/tspi-cam/actiond.toml"
else
    echo "config already exists, not overwritten: /etc/tspi-cam/actiond.toml"
fi

install -d /userdata/rec
install -m 0644 deploy/systemd/actiond.service /etc/systemd/system/actiond.service
systemctl daemon-reload
systemctl enable actiond.service
echo "installed. Start with: sudo systemctl start actiond"
