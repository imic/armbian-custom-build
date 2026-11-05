#!/bin/bash
# Enable QCA6390 WLAN and BT by driving enable GPIOs high (tlmm pins 20 = WLAN, 21 = BT).
# TLMM is f100000.pinctrl. The qca639x driver does not probe, so we assert enables in userspace.

set -e
WLAN_LINE=20
BT_LINE=21

# Find TLMM gpiochip (f100000.pinctrl) via gpiodetect
find_tlmm_chip() {
	local line
	while IFS= read -r line; do
		if [[ "$line" == *"f100000.pinctrl"* ]]; then
			# e.g. "gpiochip3 [f100000.pinctrl] (230 lines)"
			echo "$line" | sed -n 's/^\(gpiochip[0-9]*\).*/\1/p'
			return 0
		fi
	done < <(gpiodetect 2>/dev/null || true)
	return 1
}

chip=$(find_tlmm_chip)
if [[ -z "$chip" ]]; then
	echo "qca6390-wlan-enable: gpiodetect did not find f100000.pinctrl; install gpiod and check gpiodetect" >&2
	exit 1
fi

dev="/dev/${chip}"
if [[ ! -c "$dev" ]]; then
	echo "qca6390-wlan-enable: $dev not found" >&2
	exit 1
fi

# Hold both lines high (gpioset runs until reboot). Rescan PCI so WLAN/BT appear.
nohup gpioset "$dev" "${WLAN_LINE}=1" "${BT_LINE}=1" </dev/null >/dev/null 2>&1 &
sleep 1
echo 1 > /sys/bus/pci/rescan 2>/dev/null || true

# Wait for a wireless interface to appear (qca6390 shows as wlan0) so NetworkManager sees it when it starts.
# Without this, NM may start before the PCI device is bound and never see WiFi on USB-host images.
for i in $(seq 1 25); do
	for iface in /sys/class/net/wl*; do
		[[ -e "$iface" ]] && exit 0
	done
	sleep 0.2
done
# Timeout: exit anyway so boot is not stuck; WiFi may still appear shortly after.
exit 0
