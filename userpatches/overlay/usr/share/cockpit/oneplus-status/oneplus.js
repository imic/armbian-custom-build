"use strict";

/* OnePlus 8T status page: poll collect.sh every 3 s and render. */

const COLLECT_CMD =
    'for p in "$HOME/.local/share/cockpit/oneplus-status/collect.sh" ' +
    '"/usr/share/cockpit/oneplus-status/collect.sh"; do ' +
    '[ -x "$p" ] && exec "$p"; done; echo "collector_missing=1"';

let running = false;

function setText(id, text) {
    const el = document.getElementById(id);
    if (el) el.textContent = (text === undefined || text === null || text === "" || text === "-") ? "–" : text;
}

function num(v) { const n = parseFloat(v); return isNaN(n) ? null : n; }

function fmtTempMilli(v) { const n = num(v); return n === null ? null : (n / 1000).toFixed(1) + " °C"; }
function fmtVoltUv(v) { const n = num(v); return n === null ? null : (n / 1000000).toFixed(2) + " V"; }
function fmtCurrUa(v) { const n = num(v); return n === null ? null : (n >= 0 ? "+" : "") + Math.round(n / 1000) + " mA"; }
function fmtKhz(v) { const n = num(v); return n === null ? null : Math.round(n / 1000) + " MHz"; }
function fmtHz(v) { const n = num(v); return n === null ? null : Math.round(n / 1000000) + " MHz"; }
function fmtGb(kb) { const n = num(kb); return n === null ? null : (n / 1048576).toFixed(1) + " GB"; }

function render(d) {
    setText("f-wifi_ssid", d.wifi_ssid);
    setText("f-wifi_state", d.wifi_state);
    setText("f-wifi_ip", d.wifi_ip);
    const f = num(d.wifi_freq);
    setText("f-wifi_freq", f === null ? null : f.toFixed(0) + " MHz" + (f >= 5000 ? " · 5 GHz" : " · 2.4 GHz"));
    setText("f-wifi_signal", d.wifi_signal ? d.wifi_signal + " dBm" : null);

    setText("f-bt_addr", d.bt_addr);
    setText("f-bt_powered", d.bt_powered === "yes" ? "已开启" : (d.bt_powered === "no" ? "关闭" : null));
    const bc = num(d.bt_conn_count);
    setText("f-bt_conn", bc === null ? null : (bc === 0 ? "无" : bc + " 台" + (d.bt_conn_names ? "（" + d.bt_conn_names + "）" : "")));

    setText("f-nfc_present", d.nfc_present === "yes" ? "在位 (nfc0)" : (d.nfc_present === "no" ? "缺失" : null));
    setText("f-nfc_watch", d.nfc_watch === "active" ? "运行中" : d.nfc_watch);

    const cap = num(d.batt_capacity);
    setText("f-batt_capacity", cap === null ? "–" : cap.toFixed(0));
    const bar = document.getElementById("f-batt_bar");
    if (bar && cap !== null) bar.style.width = Math.max(0, Math.min(100, cap)) + "%";
    setText("f-batt_status", d.batt_status);
    setText("f-batt_voltage", fmtVoltUv(d.batt_voltage));
    setText("f-batt_current", fmtCurrUa(d.batt_current));
    const bt = num(d.batt_temp);
    const btText = bt === null ? null : (bt / 10).toFixed(1) + " °C";
    setText("f-batt_temp", btText);
    setText("f-temp_batt", btText);
    setText("f-batt_health", d.batt_health && d.batt_health !== "-" ? d.batt_health + "%" : null);
    const cf = num(d.batt_full), cd = num(d.batt_design);
    setText("f-batt_full", cf === null || cd === null ? null : Math.round(cf / 1000) + " / " + Math.round(cd / 1000) + " mAh");
    setText("f-batt_cycles", d.batt_cycles);

    setText("f-chg_online", d.chg_online === "1" ? "已接入" : (d.chg_online === "0" ? "未接入" : null));
    setText("f-chg_status", d.chg_status);
    setText("f-chg_vbus", fmtVoltUv(d.chg_vbus));
    setText("f-chg_iin", fmtCurrUa(d.chg_iin));
    setText("f-chg_usb_type", d.chg_usb_type || d.chg_usb_type_raw);
    setText("f-chg_enabled", d.chg_enabled === "1" ? "开（允许充电）" : (d.chg_enabled === "0" ? "关（充限已停充）" : null));

    setText("f-temp_cpu", fmtTempMilli(d.temp_cpu));
    setText("f-temp_gpu", fmtTempMilli(d.temp_gpu));
    setText("f-temp_pmic", fmtTempMilli(d.temp_pmic));
    setText("f-temp_aoss", fmtTempMilli(d.temp_aoss));

    setText("f-load1", d.load1);
    setText("f-cpu0_freq", fmtKhz(d.cpu0_freq));
    setText("f-cpu4_freq", fmtKhz(d.cpu4_freq));
    setText("f-cpu7_freq", fmtKhz(d.cpu7_freq));
    setText("f-gpu_freq", fmtHz(d.gpu_freq));
    setText("f-gpu_cool", d.gpu_cool_cur !== undefined ? d.gpu_cool_cur + " / " + d.gpu_cool_max + " 档" : null);

    const mt = num(d.mem_total), ma = num(d.mem_avail);
    setText("f-mem", mt === null || ma === null ? null : fmtGb(ma) + " 可用 / " + fmtGb(mt));
    const st = num(d.swap_total), sf = num(d.swap_free);
    setText("f-swap", st === null || sf === null ? null : fmtGb(st - sf) + " / " + fmtGb(st));
    const zo = num(d.zram_orig), zc = num(d.zram_compr);
    setText("f-zram", zo === null || zc === null || zo < 4096 ? "未使用" : (zo / Math.max(1, zc)).toFixed(1) + "×");
    const bc2 = num(d.bl_cur), bm = num(d.bl_max);
    setText("f-bl", bc2 === null || bm === null || bm === 0 ? null : Math.round(bc2 * 100 / bm) + "%");
    const up = num(d.uptime_s);
    if (up !== null) {
        const days = Math.floor(up / 86400), hrs = Math.floor(up % 86400 / 3600), min = Math.floor(up % 3600 / 60);
        setText("f-uptime", (days ? days + " 天 " : "") + hrs + " 小时 " + min + " 分");
    }

    const now = new Date();
    setText("op-updated", "更新于 " + now.toLocaleTimeString("zh-CN", { hour12: false }) + " · 每 3 秒刷新");
}

function refresh() {
    if (running) return;
    running = true;
    cockpit.spawn(["/bin/sh", "-c", COLLECT_CMD], { err: "message" })
        .then(function (out) {
            const d = {};
            out.split("\n").forEach(function (line) {
                const i = line.indexOf("=");
                if (i > 0) d[line.slice(0, i)] = line.slice(i + 1).trim();
            });
            const err = document.getElementById("op-error");
            if (d.collector_missing) {
                err.hidden = false;
                err.textContent = "找不到 collect.sh（插件文件不完整）";
            } else {
                err.hidden = true;
                render(d);
            }
        })
        .catch(function (e) {
            const err = document.getElementById("op-error");
            err.hidden = false;
            err.textContent = "采集失败：" + (e && e.message ? e.message : e);
        })
        .finally(function () { running = false; });
}

document.addEventListener("DOMContentLoaded", function () {
    refresh();
    setInterval(refresh, 3000);
});
