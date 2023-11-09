sudo cp target/debug/pstate_update /usr/local/bin/
sudo cp pstate_update.service /etc/systemd/system/
sudo systemctl daemon-reload
