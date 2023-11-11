#!/usr/bin/env bash
set -euxo pipefail

cargo build --release
sudo cp target/release/pstate_update /usr/local/bin/
sudo mkdir /etc/pstate_update
sudo cp config.toml /etc/pstate_update/
sudo cp pstate_update.service /etc/systemd/system/
sudo systemctl daemon-reload
