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

# The overlay arrives with the build runner's uid on every path
# (the checkout/rsync chain preserves it), and cp -a stamps that
# ownership onto the existing system directories it merges into -
# /etc and /usr ended up owned by uid 1001. Stock daemons run with
# full capabilities and never notice, but timedated's unit caps it
# to CAP_SYS_TIME (no CAP_DAC_OVERRIDE), so even as root it cannot
# write /etc/localtime and every real timezone change fails with
# EACCES (bisected on-device 2026-11-10; the no-op case never
# reaches the write - timedated returns early when the requested
# zone already matches). Everything the overlay ships is meant to
# be root-owned: normalize exactly the paths the overlay contains,
# nothing else.
if [ -d /tmp/overlay ]; then
	(cd /tmp/overlay && find . -exec chown root:root '/{}' \;)
fi

# nfctool (the NFC card tool, source in tools/nfctool/) travels
# base64-encoded as /usr/local/bin/nfctool.b64: the channel that
# pushes this repo only carries UTF-8 text, so the musl-static
# binary is stored encoded and materialized here. Hash-verified -
# a mismatch fails the build rather than shipping a corrupt tool.
if [ -f /usr/local/bin/nfctool.b64 ]; then
	base64 -d /usr/local/bin/nfctool.b64 > /usr/local/bin/nfctool
	rm -f /usr/local/bin/nfctool.b64
	chmod +x /usr/local/bin/nfctool
	echo "42e7827a5b1a370419242a7cc0512bf71d97a71e2034f28901721966a66b45a0  /usr/local/bin/nfctool" | sha256sum -c - > /dev/null || { echo "customize: nfctool sha256 mismatch after decode" >&2; exit 1; }
fi

apt-get update
# System packages only - the image is always minimal now. The Xfce
# desktop is NOT baked in (user ruling 2026-11-06): even with
# --install-recommends, the build-chroot install resolved to a
# noticeably heavier set than the hand install on the reference
# device, so the desktop installs on the device itself via
# /usr/local/bin/install-desktop.sh - a plain apt install on the
# running system, exactly like the manual install it replaces. The
# workflow's desktop input and the /etc/armbian-variant marker no
# longer change anything here.
# polkitd is deliberately NOT installed (user ruling 2026-11-06,
# stock-image parity): with it present, timedated denies the
# firstrun timezone step - timedate1's default policy is
# auth_admin_keep and a caller in an SSH session has no auth agent
# to answer (live A/B on run #8: polkitd up -> set-timezone denied
# rc=1; polkitd stopped -> rc=0). With no polkitd, root callers
# pass, exactly like the stock image. If a desktop later needs GUI
# elevation, the polkit agent packages (policykit-1-gnome etc.)
# depend on polkitd and pull it back in automatically.
# bluez: without it there is no Bluetooth userspace at all - the
# kernel brings hci0 up (QCA6390 setup completes) but nothing can
# pair or connect.
apt-get install -y bluez gpiod wpasupplicant dhcpcd-base

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
# load on a fresh image. (The GPU zap firmware is referenced by
# its qcom/ path directly and needs no copy.)
install -d /lib/firmware/postmarketos
cp /lib/firmware/qcom/sm8250/OnePlus/adsp.mbn /lib/firmware/postmarketos/adsp.mbn
cp /lib/firmware/qcom/sm8250/OnePlus/cdsp.mbn /lib/firmware/postmarketos/cdsp.mbn
cp /lib/firmware/qcom/sm8250/OnePlus/kebab/venus.mbn /lib/firmware/postmarketos/venus.mbn
# SLPI is deliberately NOT materialized (disabled 2026-11-08):
# the blob boots, but its sensor_process dies on its first QDI
# invoke - mainline has no host-side QDI service - and the PAS
# recovery loop then crash-restarts it ~3.6x/s forever, hanging
# the whole system (measured on-device: crash #714 at t=199 s,
# SSH/WiFi dying under the restart storm). With no slpi.mbn in
# the image the driver fails once with ENOENT and stays quiet -
# the #12 arrangement. The overlay .b64 was removed with the
# decode block, and xz-utils (added only for that decode) left
# the install list with it. Revisit only if mainline grows the
# QDI host service.

# Retired 2026-11-06 (units/scripts stay in the overlay, dormant):
# - wifi-join.service (+ the qca6390-wlan-enable.service it pulled
#   in via Wants=): WiFi is owned by netplan, configured in the
#   firstlogin wizard - the arrangement that actually works since
#   run #7. wifi-join's /root/wpa.conf path is gone with it.
# - bt-fixed-mac.service: its bluetoothctl mgmt.public-addr call
#   forces a QCA re-setup that wedged the controller on run #7,
#   and the chip's NVM default address (00:00:00:00:5A:AD) is
#   stable anyway, so the fixed address bought nothing.
# - nfc-watch.service (+ nfc-watch.py): the neard + python watcher
#   pair is replaced by nfctool's own watch mode (nfctool-watch
#   below) - the D-Bus handover between neard and the reader
#   proved brittle (neard loses the adapter after a peer power
#   cycle), and the watcher's Gio dependency (python3-gi) left
#   the package set with it. Files stay in the overlay, dormant;
#   reviving them needs python3-gi reinstalled first.
systemctl enable charge-limit.timer
systemctl enable nfctool-watch.service
systemctl enable panel-blank.service
systemctl enable volkey-brightness.service
# wlan-factory-mac.service: mainline ath11k never reads the
# persist partition, so the WLAN interface is born with a
# board-data template address that changes every boot, and the
# network stack then fights over it (a supplicant/kernel address
# mismatch wedges association; DHCP churns). The unit reads the
# unit's own factory address (Intf0MacAddress in
# qca6390/wlan_mac.bin on persist - what the stock qca driver
# reads) and publishes it as a udev .link file, so the interface
# is born with it at device creation, before any service starts
# (an ip link set from a service always loses the race - the
# interface is already up by then and ath11k refuses). The
# generated .link also carries the interface name: the first
# matching .link wins wholesale, so it must restate wlp1s0 or
# the interface keeps its kernel name and netplan misses it.
# Fail-open: a missing or malformed file leaves the address
# untouched. No per-unit data is baked in - each unit reads its
# own persist at boot.
systemctl enable wlan-factory-mac.service
# bt-provision.service is bt-fixed-mac's replacement (2026-11-06):
# the QCA controller registers unconfigured and only becomes usable
# after a public address is set over mgmt; bt-provision does exactly
# that - safe now that the PMU power sequencer owns the BT enable
# line and the re-setup it triggers can really power-cycle the
# controller. It runs in the boot transaction itself: the script
# feeds btmgmt a held-open stdin (btmgmt never returns with the
# /dev/null stdin a service normally gets - the 2026-11-10
# bisection; see the script header), and with that fixed it
# provisions seconds after hci0 registers (measured on-device
# 2026-11-10: started at boot t=7.8s, provisioned by t=11s).
# The 4-minute bt-provision.timer detour of the same day is
# retired - the delay only ever guarded against the stdin hang.
systemctl enable bt-provision.service
# Same two enables the board hook performs with its package set.
systemctl enable qbootctl.service
systemctl enable usbgadget-rndis.service
# neard autostart off (see the nfc-watch retirement note above):
# the pinned neard package stays installed for manual use, but
# the adapter sits unowned by default - nfctool (watch included)
# takes it on demand.
systemctl disable neard.service 2>/dev/null || true

systemctl set-default multi-user.target
# The desktop is on-demand only (user ruling 2026-11-06): lightdm is
# installed afterwards via install-desktop.sh; it must not start at boot or
# pull the graphical target in - a headless box has no use for an X
# server sitting on a blank panel. Start it by hand when a local
# desktop is wanted: systemctl start lightdm. xrdp is unaffected
# (it spawns its own X per connection).
systemctl disable lightdm.service 2>/dev/null || true

# The generic image enables serial-getty@ttyS0, but this board has
# no /dev/ttyS0 at all (no serial console device is created), so
# every boot waits ~90 s for dev-ttyS0.device to time out before
# the getty's dependency fails - on first boot it also holds up
# the firstlogin wizard's "system finished booting" gate. Mask it
# (observed on-device 2026-11-10).
systemctl mask serial-getty@ttyS0.service

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
