#!/bin/bash
# Runs chrooted in the target rootfs at the end of the armbian build.
# Ships the desktop stack and services this device's rootfs carries,
# so a freshly built image boots into the same state as the reference
# device. Per-device data (WiFi credentials, MAC pins, SSH keys, NFC
# card library) is NOT part of the image; see README.zh-CN.md.
set -e
export DEBIAN_FRONTEND=noninteractive

apt-get update
apt-get install -y xfce4 lightdm xrdp xorgxrdp neard gpiod wpasupplicant dhcpcd-base

# Files committed through the GitHub API lose their executable bit;
# restore it for the programs the overlay ships.
chmod +x /usr/local/bin/* 2>/dev/null || true

# qca6390-wlan-enable.service is intentionally not enabled here:
# wifi-join.service pulls it in via Wants= at boot.
systemctl enable charge-limit.timer
systemctl enable nfc-watch.service
systemctl enable panel-blank.service
systemctl enable volkey-brightness.service
systemctl enable wifi-join.service
systemctl enable bt-fixed-mac.service

systemctl set-default multi-user.target
