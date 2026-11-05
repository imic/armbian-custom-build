#!/bin/sh
# Install the Xfce desktop on the running system - a plain apt
# install on the live device, exactly like the hand install on the
# reference device. The desktop is deliberately NOT baked into the
# image: the build-chroot install resolved to a heavier package set
# than this same install on a running system.
#
# Needs root and network. Afterwards the desktop stays on-demand
# (lightdm disabled, multi-user target): start it by hand with
#   systemctl start lightdm
# or just connect via xrdp.
set -e

apt-get update
apt-get install -y xfce4 lightdm xrdp xorgxrdp

# Optional extras, not part of the manual-install core set:
# Bluetooth pairing GUI:
#   apt-get install -y blueman
# Chinese set (CJK fonts, zh-hans language pack, fcitx5 Pinyin):
#   apt-get install -y fonts-noto-cjk language-pack-zh-hans fcitx5 \
#       fcitx5-chinese-addons fcitx5-frontend-gtk3 fcitx5-frontend-qt5
#   im-config -n fcitx5

systemctl disable lightdm.service 2>/dev/null || true
systemctl set-default multi-user.target

echo "Desktop installed (on-demand). Start it with: systemctl start lightdm"
