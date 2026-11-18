#!/usr/bin/python3
# Charge hysteresis on the MPS charger (bus5 0x5c), v3: decisions on sag-compensated
# resting voltage vRest = V - I*R. Park at vRest 8.12 V (runtime priority, user
# 2026-11-14), resume at vRest 7.85 V, hard stop at raw loaded 8.68 V (charger CV
# is 8.72 V). R is auto-calibrated per unit: while charging in the working band,
# the script briefly clears CHG_EN and takes R = dV/dI across the step, keeping the
# median of the last 5 accepted samples (state in /var/lib/mp2650-charge-limit).
# Until the first accepted sample, R_DEFAULT applies. (v2 used a fixed 0.97 ohm,
# hand-measured 2026-11-14 on charge and discharge legs of one unit.)
import sys, os, time, fcntl, ctypes
BUS = "/dev/i2c-5"; ADDR = 0x5C
REG_IINLIM = 0x00; REG_ICHG = 0x02; REG_CFG = 0x08; CHG_EN = 0x10
TARGET_IINLIM = 0x28
TARGET_ICHG = 0x14
STOP_REST_UV = 8120000; RESUME_REST_UV = 7850000; RAW_CAP_UV = 8680000
R_DEFAULT = 0.5
R_MIN = 0.05; R_MAX = 3.0
STATE_DIR = "/var/lib/mp2650-charge-limit"; STATE_FILE = STATE_DIR + "/r_samples"
PROBE_MAX_AGE_S = 7 * 24 * 3600
PROBE_MIN_I_UA = 350000; PROBE_MIN_DI_UA = 300000; PROBE_STABLE_UA = 40000
PROBE_MIN_V_UV = 7300000
GAUGE_V = "/sys/class/power_supply/bq27541-0/voltage_now"
GAUGE_I = "/sys/class/power_supply/bq27541-0/current_now"
I2C_RDWR = 0x0707
class Msg(ctypes.Structure):
    _fields_ = [("addr", ctypes.c_ushort), ("flags", ctypes.c_ushort), ("len", ctypes.c_ushort), ("buf", ctypes.POINTER(ctypes.c_ubyte))]
class Data(ctypes.Structure):
    _fields_ = [("msgs", ctypes.POINTER(Msg)), ("nmsgs", ctypes.c_uint)]
fd = open(BUS, "r+b", buffering=0)
def rd(reg):
    wb = (ctypes.c_ubyte * 1)(reg); rb = (ctypes.c_ubyte * 1)()
    m = (Msg * 2)(Msg(ADDR, 0, 1, wb), Msg(ADDR, 1, 1, rb))
    fcntl.ioctl(fd, I2C_RDWR, Data(m, 2)); return rb[0]
def wr(reg, val):
    wb = (ctypes.c_ubyte * 2)(reg, val)
    m = (Msg * 1)(Msg(ADDR, 0, 2, wb))
    fcntl.ioctl(fd, I2C_RDWR, Data(m, 1))
def gauge():
    return (int(open(GAUGE_V).read().strip()), int(open(GAUGE_I).read().strip()))
def load_samples():
    try:
        out = []
        for line in open(STATE_FILE):
            t, r = line.split()
            out.append((int(t), float(r)))
        return out[-5:]
    except Exception:
        return []
def save_sample(r):
    os.makedirs(STATE_DIR, exist_ok=True)
    samples = load_samples() + [(int(time.time()), round(r, 3))]
    with open(STATE_FILE, "w") as f:
        for t, val in samples[-5:]:
            f.write("%d %.3f\n" % (t, val))
def eff_r(samples):
    if not samples:
        return R_DEFAULT
    vals = sorted(r for _, r in samples)
    return vals[len(vals) // 2]
def probe_due(samples):
    if not samples:
        return True
    return (time.time() - samples[-1][0]) > PROBE_MAX_AGE_S
def settled_reads():
    # Poll until the gauge's filtered V/I stop moving. Its current average
    # lags a real step by ~20 s (measured 2026-11-16: +600 mA reading decayed
    # ~35 mA/s after CHG_EN cleared), so single reads after a step are wrong.
    pv, pi = gauge()
    for _ in range(20):
        time.sleep(2.0)
        v, i = gauge()
        if abs(v - pv) <= 2000 and abs(i - pi) <= 10000:
            return (v, i)
        pv, pi = v, i
    return (pv, pi)
def probe(v, i):
    # Returns an accepted R estimate or None. Caller guarantees charging is
    # on. Take settled reads on both sides of a CHG_EN step: R = dV/dI.
    v1, i1 = settled_reads()
    if i1 < PROBE_MIN_I_UA:
        return None
    cfg = rd(REG_CFG)
    try:
        wr(REG_CFG, cfg & ~CHG_EN)
        v2, i2 = settled_reads()
    finally:
        wr(REG_CFG, rd(REG_CFG) | CHG_EN)
    di = i1 - i2
    if di < PROBE_MIN_DI_UA:
        return None
    r = (v1 - v2) / di
    if R_MIN <= r <= R_MAX:
        return r
    return None
def main():
    mode = sys.argv[1] if len(sys.argv) > 1 else "auto"
    notes = []
    samples = load_samples()
    r_ohm = eff_r(samples)
    v, i = gauge()
    if rd(REG_IINLIM) != TARGET_IINLIM:
        wr(REG_IINLIM, TARGET_IINLIM); notes.append("iinlim->2.0A")
    if rd(REG_ICHG) != TARGET_ICHG:
        wr(REG_ICHG, TARGET_ICHG); notes.append("ichg->1.0A")
    cfg = rd(REG_CFG); on = bool(cfg & CHG_EN)
    if (mode == "auto" and on and probe_due(samples)
            and i >= PROBE_MIN_I_UA
            and v >= PROBE_MIN_V_UV):
        got = probe(v, i)
        if got is not None:
            save_sample(got)
            samples = load_samples(); r_ohm = eff_r(samples)
            notes.append("r-probe=%.3f" % got)
        v, i = gauge()
        cfg = rd(REG_CFG); on = bool(cfg & CHG_EN)
    vrest = int(v - i * r_ohm)
    want = on
    if mode == "--off": want = False
    elif mode == "--on": want = True
    elif on and (vrest >= STOP_REST_UV or v >= RAW_CAP_UV): want = False
    elif not on and vrest <= RESUME_REST_UV: want = True
    if want != on:
        wr(REG_CFG, (cfg | CHG_EN) if want else (cfg & ~CHG_EN))
        notes.append("chg_en->" + ("on" if want else "off"))
    print("mp2650-limit V=%.3f I=%+d vRest=%.3f(R=%.2f) chg_en=%s %s" % (v / 1000000.0, i, vrest / 1000000.0, r_ohm, "on" if want else "off", " ".join(notes) or "no-change"))
try:
    main()
except Exception as e:
    print("mp2650-limit ERROR:", e); sys.exit(1)
