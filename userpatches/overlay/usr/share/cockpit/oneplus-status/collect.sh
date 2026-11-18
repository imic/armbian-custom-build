#!/bin/sh
# OnePlus 8T status collector for the Cockpit plugin.
# Prints KEY=VALUE lines. Everything read here is world-readable sysfs
# or unprivileged user commands on this device (verified 2026-11-11).
# Missing sources print "-" instead of failing.

r() { cat "$1" 2>/dev/null || echo "-"; }

# ---- WiFi (ath11k, wlp1s0) ----
IF=wlp1s0
echo "wifi_state=$(r /sys/class/net/$IF/operstate)"
LINK=$(iw dev $IF link 2>/dev/null)
echo "wifi_ssid=$(echo "$LINK" | sed -n 's/.*SSID: //p' | head -1)"
echo "wifi_freq=$(echo "$LINK" | sed -n 's/.*freq: \([0-9.]*\).*/\1/p' | head -1)"
echo "wifi_signal=$(echo "$LINK" | sed -n 's/.*signal: \(-\{0,1\}[0-9]*\).*/\1/p' | head -1)"
echo "wifi_ip=$(ip -4 -brief addr show $IF 2>/dev/null | awk '{print $3}')"

# ---- Bluetooth ----
SHOW=$(bluetoothctl show 2>/dev/null)
echo "bt_addr=$(echo "$SHOW" | sed -n 's/.*Controller \([0-9A-F:]*\).*/\1/p' | head -1)"
echo "bt_powered=$(echo "$SHOW" | grep -q 'Powered: yes' && echo yes || echo no)"
CONN=$(bluetoothctl devices Connected 2>/dev/null)
echo "bt_conn_count=$(echo "$CONN" | grep -c '^Device ')"
echo "bt_conn_names=$(echo "$CONN" | sed -n 's/^Device [0-9A-F:]* //p' | paste -sd ',' -)"

# ---- NFC ----
[ -e /sys/class/nfc/nfc0 ] && echo "nfc_present=yes" || echo "nfc_present=no"
echo "nfc_watch=$(systemctl is-active nfctool-watch.service 2>/dev/null || echo unknown)"

# ---- Battery (bq27541 gauge is authoritative on this device) ----
G=/sys/class/power_supply/bq27541-0
echo "batt_capacity=$(r $G/capacity)"
echo "batt_status=$(r $G/status)"
echo "batt_voltage=$(r $G/voltage_now)"
echo "batt_current=$(r $G/current_now)"
echo "batt_temp=$(r $G/temp)"
echo "batt_cycles=$(r $G/cycle_count)"
CF=$(r $G/charge_full); CD=$(r $G/charge_full_design)
if [ "$CF" != "-" ] && [ "$CD" != "-" ] && [ "$CD" != "0" ]; then
    echo "batt_health=$((CF * 100 / CD))"
else
    echo "batt_health=-"
fi
echo "batt_full=$CF"
echo "batt_design=$CD"

# ---- Charger (MP2650 is the real charger; PM8150B ADC senses USB-IN) ----
M=/sys/class/power_supply/mp2650-charger
echo "chg_online=$(r $M/online)"
echo "chg_status=$(r $M/status)"
ADC=""
for d in /sys/bus/iio/devices/iio:device*; do
    [ -e "$d/in_voltage_usb_in_v_div_16_input" ] && ADC=$d && break
done
if [ -n "$ADC" ]; then
    echo "chg_vbus=$(r $ADC/in_voltage_usb_in_v_div_16_input)"
else
    echo "chg_vbus=-"
fi
# No trustworthy measured input current exists on this board: the
# charge current flows through the MP2650 (no telemetry), and the
# PMIC current channel does not carry it (reads 0.30 A unplugged,
# 0 while charging). Show the programmed input limit instead.
echo "chg_iin=$(r $M/input_current_limit)"
CB=$(r $M/charge_behaviour)
case "$CB" in
    *"[auto]"*) echo "chg_enabled=1" ;;
    *inhibit-charge*) echo "chg_enabled=0" ;;
    *) echo "chg_enabled=-" ;;
esac
# Link type from the TCPM port (pm8150b usb_type is APSD noise; do not use)
if [ "$(r $M/online)" = "1" ]; then
    # op_mode alone can be stale after a hot swap; PD counts only when
    # the attached partner actually negotiated it.
    SPD=$(r /sys/class/typec/port0-partner/supports_usb_power_delivery)
    OM=$(r /sys/class/typec/port0/power_operation_mode)
    if [ "$SPD" = "yes" ] && [ "$OM" = "usb_power_delivery" ]; then
        TIER=""
        VB=$(r $ADC/in_voltage_usb_in_v_div_16_input)
        if [ -n "$ADC" ] && [ "$VB" != "-" ]; then
            for p in /sys/class/typec/port0/usb_power_delivery/sink-capabilities/*/; do
                pv=$(cat $p/voltage 2>/dev/null | tr -d 'mV')
                pa=$(cat $p/operational_current 2>/dev/null | tr -d 'mA')
                [ -z "$pv" ] && continue
                d=$(( VB / 1000 - pv )); [ $d -lt 0 ] && d=$(( -d ))
                [ $d -lt 600 ] && TIER=" $((pv / 1000))V/${pa}mA"
            done
        fi
        echo "chg_usb_type=USB PD$TIER"
    else
        echo "chg_usb_type=USB (5V)"
    fi
else
    echo "chg_usb_type=-"
fi

# ---- Thermal: pick the meaningful zones by type name ----
tmax() { # $1 = grep -E pattern over zone types
    m=""
    for z in /sys/class/thermal/thermal_zone*; do
        t=$(cat "$z/type" 2>/dev/null) || continue
        echo "$t" | grep -qE "$1" || continue
        v=$(cat "$z/temp" 2>/dev/null) || continue
        [ -z "$m" ] || [ "$v" -gt "$m" ] && m=$v
    done
    echo "${m:--}"
}
echo "temp_cpu=$(tmax '^cpu[0-9]')"
echo "temp_gpu=$(tmax '^gpu')"
echo "temp_pmic=$(tmax '^pm8150')"
echo "temp_aoss=$(tmax '^aoss0')"

# ---- CPU clusters (little cpu0 / big cpu4 / prime cpu7 representatives) ----
echo "cpu0_freq=$(r /sys/devices/system/cpu/cpu0/cpufreq/scaling_cur_freq)"
echo "cpu4_freq=$(r /sys/devices/system/cpu/cpu4/cpufreq/scaling_cur_freq)"
echo "cpu7_freq=$(r /sys/devices/system/cpu/cpu7/cpufreq/scaling_cur_freq)"
echo "load1=$(awk '{print $1}' /proc/loadavg 2>/dev/null)"

# ---- GPU (Adreno 650, devfreq) ----
echo "gpu_freq=$(r /sys/class/devfreq/3d00000.gpu/cur_freq)"
for c in /sys/class/thermal/cooling_device*; do
    [ "$(cat $c/type 2>/dev/null)" = "devfreq-3d00000.gpu" ] || continue
    echo "gpu_cool_cur=$(cat $c/cur_state 2>/dev/null)"
    echo "gpu_cool_max=$(cat $c/max_state 2>/dev/null)"
done

# ---- Memory + zram ----
while read -r k v _; do
    case "$k" in
        MemTotal:)  echo "mem_total=$v" ;;
        MemAvailable:) echo "mem_avail=$v" ;;
        SwapTotal:) echo "swap_total=$v" ;;
        SwapFree:)  echo "swap_free=$v" ;;
    esac
done < /proc/meminfo
echo "zram_orig=$(awk '{print $1}' /sys/block/zram0/mm_stat 2>/dev/null || echo -)"
echo "zram_compr=$(awk '{print $2}' /sys/block/zram0/mm_stat 2>/dev/null || echo -)"

# ---- Backlight ----
B=$(ls /sys/class/backlight/ 2>/dev/null | head -1)
if [ -n "$B" ]; then
    echo "bl_cur=$(r /sys/class/backlight/$B/brightness)"
    echo "bl_max=$(r /sys/class/backlight/$B/max_brightness)"
fi

echo "uptime_s=$(awk '{printf "%d", $1}' /proc/uptime 2>/dev/null)"

# ---- Storage: the rootfs lives on the GPT "linux" partition ----
echo "stor_dev=$(readlink -f /dev/disk/by-partlabel/linux 2>/dev/null || echo -)"
set -- $(df -B1 --output=size,used,avail,pcent / 2>/dev/null | tail -1)
echo "stor_size=${1:-}"
echo "stor_used=${2:-}"
echo "stor_avail=${3:-}"
echo "stor_pcent=${4%\%}"
