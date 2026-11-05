#!/bin/bash
# Runs chrooted in the target rootfs at the end of the armbian build.
# Ships the desktop stack and services this device's rootfs carries,
# so a freshly built image boots into the same state as the reference
# device. Per-device data (WiFi credentials, MAC pins, SSH keys, NFC
# card library) is NOT part of the image; see README.zh-CN.md.
set -e
export DEBIAN_FRONTEND=noninteractive

# The framework bind-mounts userpatches/overlay read-only at
# /tmp/overlay for this script; it does NOT copy it into the image
# itself (lib/functions/rootfs/customize.sh). Apply it now, so the
# units and scripts below exist when they are enabled/chmod'ed.
# (The desktop-variant marker, when present, arrives this way too.)
if [ -d /tmp/overlay ]; then
	cp -a /tmp/overlay/. /
fi

apt-get update
if [ -f /etc/armbian-variant ] && grep -qx minimal /etc/armbian-variant; then
	# "desktop: no" was selected when dispatching the workflow: the
	# marker file above was placed into the overlay by the workflow.
	# Device-function packages still install; the desktop stack is
	# skipped, leaving a pure minimal image.
	# polkitd even without a desktop: timedated & friends ask it for
	# authorization, and with no daemon the firstrun timezone step is
	# denied even for root.
	# bluez in both variants too: bt-fixed-mac.service (enabled for
	# both) drives bluetoothctl, and without the bluez package there
	# is no Bluetooth userspace at all - the kernel brings hci0 up
	# (QCA6390 setup completes) but nothing can pair or connect.
	apt-get install -y polkitd bluez gpiod wpasupplicant dhcpcd-base
else
	# --install-recommends: the build chroot's apt skips Recommends,
	# which silently drops the xfce4 metapackage's whole second tier
	# (polkit agent, terminal, notifyd, gvfs, ...) - the reference
	# device got that tier from a plain apt install on the running
	# system. polkitd is named explicitly as well: with no polkit
	# daemon the firstrun timezone step and every desktop privilege
	# action are denied. The Chinese set (CJK fonts, zh-hans language
	# pack, fcitx5 with the Pinyin engine) is named explicitly too;
	# nothing Recommends it. Same for bluez (the kernel brings hci0
	# up on its own, but pairing needs the daemon and bluetoothctl -
	# bt-fixed-mac.service calls it) and blueman, the desktop pairing
	# applet; neither is in xfce4's Recommends chain.
	apt-get install -y --install-recommends xfce4 lightdm xrdp xorgxrdp polkitd bluez blueman gpiod wpasupplicant dhcpcd-base fonts-noto-cjk language-pack-zh-hans fcitx5 fcitx5-chinese-addons fcitx5-frontend-gtk3 fcitx5-frontend-qt5
	# Make fcitx5 the system input method framework (per-user
	# im-config state would not exist on a fresh image).
	im-config -n fcitx5 || true
fi

# neard has no installation candidate in the Ubuntu resolute
# archive (it was also dropped from Debian trixie). The reference
# device runs Debian's neard 0.20-3, hand-installed from this
# exact pool .deb - its daemon binary is byte-identical to the
# reference device's installed copy. Fetch it pinned, verify the
# hash, and install locally; dependencies resolve from resolute.
curl -fsSL http://deb.debian.org/debian/pool/main/n/neard/neard_0.20-3_arm64.deb -o /tmp/neard.deb
echo "01b53d637ec554cc2af810d82dd77327785e06eae00c135e698299dc678788e3  /tmp/neard.deb" | sha256sum -c -
apt-get install -y /tmp/neard.deb
rm -f /tmp/neard.deb

# The board's own post_family_tweaks hook installs its userspace
# package set only on releases its support check lists (jammy,
# trixie, noble); resolute is not listed, so the whole set is
# missing from this build - while the reference device carries
# all of it. Install the same set here. unudhcpd is not in the
# Ubuntu archive; the board hook takes it from the armbian repo,
# temporarily enabled the same way. (At customize time the repo
# file may be present under either name, depending on build
# stage; handle both. A failed install aborts the build loudly.)
apt-get install -y alsa-ucm-conf qbootctl qrtr-tools mkbootimg dropbear-bin
if [ -f /etc/apt/sources.list.d/armbian.sources.disabled ]; then
	mv /etc/apt/sources.list.d/armbian.sources.disabled /etc/apt/sources.list.d/armbian.sources
	apt-get update
	apt-get install -y unudhcpd
	mv /etc/apt/sources.list.d/armbian.sources /etc/apt/sources.list.d/armbian.sources.disabled
	apt-get update
else
	apt-get install -y unudhcpd
fi

# Files committed through the GitHub API lose their executable bit;
# restore it for the programs the overlay ships.
chmod +x /usr/local/bin/* 2>/dev/null || true

# The device tree requests DSP firmware under postmarketos/ names,
# while the blobs ship in the BSP package under qcom/ paths. The
# copies on the reference device were hash-verified identical to
# these package files; materialize them here so ADSP/CDSP/Venus
# load on a fresh image. (SLPI stays unloaded, as on the reference
# device; the GPU zap firmware is referenced by its qcom/ path
# directly and needs no copy.)
install -d /lib/firmware/postmarketos
cp /lib/firmware/qcom/sm8250/OnePlus/adsp.mbn /lib/firmware/postmarketos/adsp.mbn
cp /lib/firmware/qcom/sm8250/OnePlus/cdsp.mbn /lib/firmware/postmarketos/cdsp.mbn
cp /lib/firmware/qcom/sm8250/OnePlus/kebab/venus.mbn /lib/firmware/postmarketos/venus.mbn

# qca6390-wlan-enable.service is intentionally not enabled here:
# wifi-join.service pulls it in via Wants= at boot.
systemctl enable charge-limit.timer
systemctl enable nfc-watch.service
systemctl enable panel-blank.service
systemctl enable volkey-brightness.service
systemctl enable wifi-join.service
systemctl enable bt-fixed-mac.service
# Same two enables the board hook performs with its package set.
systemctl enable qbootctl.service
systemctl enable usbgadget-rndis.service

systemctl set-default multi-user.target
# The desktop is on-demand only (user ruling 2026-11-06): lightdm is
# installed with the desktop variant but must not start at boot or
# pull the graphical target in - a headless box has no use for an X
# server sitting on a blank panel. Start it by hand when a local
# desktop is wanted: systemctl start lightdm. xrdp is unaffected
# (it spawns its own X per connection).
systemctl disable lightdm.service 2>/dev/null || true

# --- Slot packaging: this image is built for slot A ---------------------
# The ABL packaging extension stamps the boot image's slot_suffix from
# abl_boot_partition_label, which it sources from the rootfs's own
# /boot/armbianEnv.txt (extensions/image-output-abl.sh). The framework
# default is boot_b - its kebab image is a slot-B dual-boot product -
# while this image is flashed to and run from slot A only. A _b stamp
# on an A-slot system makes the on-device slot tooling (qbootctl)
# keep its books on the wrong slot, the mechanism behind the
# 2026-11-06 warm-reboot 900E drops.
# There is NO userpatches override for the bootenv file itself: the
# framework reads it straight from its own config/bootenv/ directory
# (lib/functions/bsp/armbian-bsp-cli-deb.sh) with no userpatches
# lookup, so the eed93414 userpatches/config/bootenv/ file never
# took effect and run #7 still shipped stamped _b. The lever that
# does work is rewriting the label in the image's own copies below -
# /boot/armbianEnv.txt is also what the on-device kernel-update
# repack (zz-update-abl-kernel) sources, so repacks stay _a too.
# Fail the build if the label is not found: a silent _b stamp is
# exactly the failure this block exists to prevent.
for envf in /boot/armbianEnv.txt /usr/share/armbian/armbianEnv.txt; do
	if [ -f "$envf" ]; then
		sed -i 's/^abl_boot_partition_label=.*/abl_boot_partition_label=boot_a/' "$envf"
	fi
done
grep -q '^abl_boot_partition_label=boot_a$' /boot/armbianEnv.txt || { echo "customize: abl_boot_partition_label is not boot_a in /boot/armbianEnv.txt - refusing to ship a _b-stamped image" >&2; exit 1; }

# --- Firmware pruning ---------------------------------------------------
# The board config installs the -full firmware tier (~2.3 GB covering
# dozens of SoCs and WiFi chips). This hardware (SM8250 + QCA6390)
# can only ever request ~105 MB of it, measured on the reference
# device: the qcom/sm8250 tree (OnePlus files only), the ath11k
# QCA6390 target, the qca Bluetooth firmware, the loose top-level
# QCA files, regulatory.db and the postmarketos/ DSP copies made
# above. Delete everything else.
FW=/lib/firmware
[ -d "$FW/qcom/sm8250" ] && [ -d "$FW/ath11k" ] || {
	echo "customize: unexpected firmware layout, refusing to prune" >&2
	exit 1
}
echo "customize: firmware before pruning: $(du -sh $FW | cut -f1)"
# (only directories: the loose files directly under qcom/ include
# this GPU's GMU firmware, qcom/a650_gmu.bin - keep those)
find $FW/qcom -mindepth 1 -maxdepth 1 -type d ! -name sm8250 -exec rm -rf {} +
find $FW/qcom/sm8250 -mindepth 1 -maxdepth 1 -type d ! -name OnePlus -exec rm -rf {} +
find $FW/ath11k -mindepth 1 -maxdepth 1 -type d ! -name QCA6390 -exec rm -rf {} +
find $FW -mindepth 1 -maxdepth 1 -type d \
	! -name qcom ! -name ath11k ! -name qca ! -name postmarketos \
	-exec rm -rf {} +
echo "customize: firmware after pruning: $(du -sh $FW | cut -f1)"
