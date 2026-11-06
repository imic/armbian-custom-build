// nfctool — MIFARE Classic card tool for the OnePlus 8T (SN100T + TAG-CMD
// kernel patch). Zero crates, raw syscalls, fully static musl binary.
// READ-ONLY against cards: the only RF commands ever sent are TAG-CMD
// AUTH and TAG-CMD READ. No write path exists in this program yet; card
// write-back is a separate, explicitly gated feature for a later version.
use std::io::Error as IoError;

const AF_NETLINK: i32 = 16;
const AF_NFC: i32 = 39;
const SOCK_RAW: i32 = 3;
const SOCK_SEQPACKET: i32 = 5;
const NETLINK_GENERIC: i32 = 16;
const GENL_ID_CTRL: u16 = 16;
const NLMSG_ERROR: u16 = 2;
const NLMSG_DONE: u16 = 3;
const NLM_F_REQUEST: u16 = 1;
const NLM_F_ACK: u16 = 4;
const NLM_F_DUMP: u16 = 0x300;
const POLLIN: i16 = 1;

const NFC_CMD_GET_DEVICE: u8 = 1;
const NFC_CMD_DEV_UP: u8 = 2;
const NFC_CMD_DEV_DOWN: u8 = 3;
const NFC_CMD_START_POLL: u8 = 6;
const NFC_CMD_STOP_POLL: u8 = 7;
const NFC_CMD_GET_TARGET: u8 = 8;

const A_DEVICE_INDEX: u16 = 1;
const A_PROTOCOLS: u16 = 3;
const A_TARGET_INDEX: u16 = 4;
const A_TARGET_SENS_RES: u16 = 5;
const A_TARGET_SEL_RES: u16 = 6;
const A_TARGET_NFCID1: u16 = 7;

const PROTO_MIFARE: u32 = 2;
const PROTO_ISO14443: u32 = 4;
const PROTO_JEWEL: u32 = 1;
const POLL_MASK: u32 = (1 << PROTO_JEWEL) | (1 << PROTO_MIFARE) | (1 << PROTO_ISO14443);

// TAG-CMD key selectors: A with key bytes = 0x10, B with key bytes = 0x90.
pub const KEY_A: u8 = 0x10;
pub const KEY_B: u8 = 0x90;
pub const FACTORY_KEY: [u8; 6] = [0xFF; 6];

#[repr(C)]
struct PollFd {
    fd: i32,
    events: i16,
    revents: i16,
}

extern "C" {
    fn socket(domain: i32, ty: i32, protocol: i32) -> i32;
    fn connect(fd: i32, addr: *const u8, len: u32) -> i32;
    fn send(fd: i32, buf: *const u8, len: usize, flags: i32) -> isize;
    fn recv(fd: i32, buf: *mut u8, len: usize, flags: i32) -> isize;
    fn poll(fds: *mut PollFd, nfds: u64, timeout: i32) -> i32;
    fn close(fd: i32) -> i32;
    fn usleep(us: u32) -> i32;
    fn signal(signum: i32, handler: usize) -> usize;
}

fn last_err() -> String {
    IoError::last_os_error().to_string()
}

fn align4(x: usize) -> usize {
    (x + 3) & !3
}

fn msg_begin(fam: u16, cmd: u8, flags: u16, seq: u32) -> Vec<u8> {
    let mut b = vec![0u8; 20];
    b[4..6].copy_from_slice(&fam.to_ne_bytes());
    b[6..8].copy_from_slice(&(NLM_F_REQUEST | flags).to_ne_bytes());
    b[8..12].copy_from_slice(&seq.to_ne_bytes());
    b[16] = cmd;
    b[17] = 1;
    b
}

fn put_attr(b: &mut Vec<u8>, ty: u16, data: &[u8]) {
    let start = b.len();
    let l = 4 + data.len();
    b.extend_from_slice(&(l as u16).to_ne_bytes());
    b.extend_from_slice(&ty.to_ne_bytes());
    b.extend_from_slice(data);
    while b.len() < start + align4(l) {
        b.push(0);
    }
}

fn finish_and_send(fd: i32, b: &mut Vec<u8>) {
    let len = b.len() as u32;
    b[0..4].copy_from_slice(&len.to_ne_bytes());
    let rc = unsafe { send(fd, b.as_ptr(), b.len(), 0) };
    if rc < 0 {
        panic!("genl send: {}", last_err());
    }
}

fn attrs_of(msg: &[u8]) -> Vec<(u16, &[u8])> {
    let mut out = Vec::new();
    if msg.len() < 20 {
        return out;
    }
    let mut off = 20usize;
    while off + 4 <= msg.len() {
        let l = u16::from_ne_bytes([msg[off], msg[off + 1]]) as usize;
        let t = u16::from_ne_bytes([msg[off + 2], msg[off + 3]]);
        if l < 4 || off + l > msg.len() {
            break;
        }
        out.push((t, &msg[off + 4..off + l]));
        off += align4(l);
    }
    out
}

fn nlmsg_type(msg: &[u8]) -> u16 {
    u16::from_ne_bytes([msg[4], msg[5]])
}

fn nlmsg_len(msg: &[u8]) -> usize {
    u32::from_ne_bytes([msg[0], msg[1], msg[2], msg[3]]) as usize
}

fn each_msg<F: FnMut(&[u8])>(buf: &[u8], mut f: F) -> bool {
    let mut off = 0usize;
    let mut done = false;
    while off + 16 <= buf.len() {
        let l = nlmsg_len(&buf[off..]);
        if l < 16 || off + l > buf.len() {
            break;
        }
        let m = &buf[off..off + l];
        if nlmsg_type(m) == NLMSG_DONE {
            done = true;
        }
        f(m);
        off += align4(l);
    }
    done
}

pub struct Nl {
    fd: i32,
    fam: u16,
}

impl Nl {
    fn recv_timeout(&self, ms: i32) -> Option<Vec<u8>> {
        let mut pfd = PollFd { fd: self.fd, events: POLLIN, revents: 0 };
        let r = unsafe { poll(&mut pfd, 1, ms) };
        if r <= 0 {
            return None;
        }
        let mut buf = vec![0u8; 16384];
        let n = unsafe { recv(self.fd, buf.as_mut_ptr(), buf.len(), 0) };
        if n < 0 {
            panic!("genl recv: {}", last_err());
        }
        buf.truncate(n as usize);
        Some(buf)
    }

    fn wait_ack(&self) -> i32 {
        for _ in 0..4 {
            let buf = match self.recv_timeout(5000) {
                Some(b) => b,
                None => return -110,
            };
            let mut ret: Option<i32> = None;
            each_msg(&buf, |m| {
                let t = nlmsg_type(m);
                if t == NLMSG_ERROR {
                    let e = i32::from_ne_bytes([m[16], m[17], m[18], m[19]]);
                    ret = Some(e);
                } else if t == NLMSG_DONE || t == self.fam {
                    ret = Some(0);
                }
            });
            if let Some(v) = ret {
                return v;
            }
        }
        -71
    }

    fn cmd_simple(&self, cmd: u8, idx: u32) -> i32 {
        let mut b = msg_begin(self.fam, cmd, NLM_F_ACK, 3);
        put_attr(&mut b, A_DEVICE_INDEX, &idx.to_ne_bytes());
        finish_and_send(self.fd, &mut b);
        self.wait_ack()
    }
}

fn resolve_family(fd: i32) -> u16 {
    let mut b = msg_begin(GENL_ID_CTRL, 3, 0, 1);
    put_attr(&mut b, 2, b"nfc\0");
    finish_and_send(fd, &mut b);
    let nl = Nl { fd, fam: 0 };
    let buf = nl.recv_timeout(5000).expect("family resolve timeout");
    let mut fam = 0u16;
    each_msg(&buf, |m| {
        if nlmsg_type(m) == NLMSG_ERROR {
            let e = i32::from_ne_bytes([m[16], m[17], m[18], m[19]]);
            panic!("GETFAMILY error {}", e);
        }
        for (t, d) in attrs_of(m) {
            if t == 1 && d.len() >= 2 {
                fam = u16::from_ne_bytes([d[0], d[1]]);
            }
        }
    });
    if fam == 0 {
        panic!("no nfc family id");
    }
    fam
}

fn get_device_idx(nl: &Nl) -> u32 {
    let mut b = msg_begin(nl.fam, NFC_CMD_GET_DEVICE, NLM_F_DUMP, 2);
    finish_and_send(nl.fd, &mut b);
    loop {
        let buf = nl.recv_timeout(5000).expect("get_device timeout");
        let mut idx: Option<u32> = None;
        let done = each_msg(&buf, |m| {
            if nlmsg_type(m) == NLMSG_ERROR {
                panic!("get_device error");
            }
            for (t, d) in attrs_of(m) {
                if t == A_DEVICE_INDEX && d.len() >= 4 {
                    idx = Some(u32::from_ne_bytes([d[0], d[1], d[2], d[3]]));
                }
            }
        });
        if let Some(i) = idx {
            return i;
        }
        if done {
            panic!("no nfc device");
        }
    }
}

#[derive(Default, Clone)]
pub struct Target {
    pub idx: u32,
    pub protocols: u32,
    pub sens_res: u16,
    pub sel_res: u8,
    pub nfcid1: Vec<u8>,
}

fn get_target(nl: &Nl, dev: u32) -> Option<Target> {
    let mut b = msg_begin(nl.fam, NFC_CMD_GET_TARGET, NLM_F_DUMP, 4);
    put_attr(&mut b, A_DEVICE_INDEX, &dev.to_ne_bytes());
    finish_and_send(nl.fd, &mut b);
    let mut found: Option<Target> = None;
    loop {
        let buf = nl.recv_timeout(3000)?;
        let done = each_msg(&buf, |m| {
            if nlmsg_type(m) == NLMSG_ERROR || nlmsg_type(m) == NLMSG_DONE {
                return;
            }
            let mut tg = Target::default();
            let mut has = false;
            for (t, d) in attrs_of(m) {
                match t {
                    A_TARGET_INDEX if d.len() >= 4 => {
                        tg.idx = u32::from_ne_bytes([d[0], d[1], d[2], d[3]]);
                        has = true;
                    }
                    A_PROTOCOLS if d.len() >= 4 => {
                        tg.protocols = u32::from_ne_bytes([d[0], d[1], d[2], d[3]]);
                    }
                    A_TARGET_SENS_RES if d.len() >= 2 => {
                        tg.sens_res = u16::from_ne_bytes([d[0], d[1]]);
                    }
                    A_TARGET_SEL_RES if !d.is_empty() => {
                        tg.sel_res = d[0];
                    }
                    A_TARGET_NFCID1 => {
                        tg.nfcid1 = d.to_vec();
                    }
                    _ => {}
                }
            }
            if has {
                found = Some(tg);
            }
        });
        if found.is_some() || done {
            return found;
        }
    }
}

/// One raw exchange on the activated rawsock. Returns the payload with
/// the kernel's 1-byte prepended header stripped.
fn xchg(fd: i32, cmd: &[u8]) -> Option<Vec<u8>> {
    let rc = unsafe { send(fd, cmd.as_ptr(), cmd.len(), 0) };
    if rc < 0 {
        return None;
    }
    let mut pfd = PollFd { fd, events: POLLIN, revents: 0 };
    if unsafe { poll(&mut pfd, 1, 3000) } <= 0 {
        return None;
    }
    let mut resp = vec![0u8; 512];
    let n = unsafe { recv(fd, resp.as_mut_ptr(), resp.len(), 0) };
    if n <= 0 {
        return None;
    }
    resp.truncate(n as usize);
    if !resp.is_empty() {
        resp.remove(0);
    }
    Some(resp)
}

/// The NFC device + one poll session. Owns the bring-up sequence the
/// probes proved: DEV_DOWN pre-clean, DEV_UP retry loop, START_POLL,
/// one sacrifice connect/close (frees the stale kernel reassembly
/// fragment), then a poll restart so targets are fresh.
pub struct Device {
    nl: Nl,
    idx: u32,
    polled_once: bool,
}

impl Device {
    pub fn open() -> Device {
        let nl_fd = unsafe { socket(AF_NETLINK, SOCK_RAW, NETLINK_GENERIC) };
        if nl_fd < 0 {
            panic!("netlink socket: {}", last_err());
        }
        let fam = resolve_family(nl_fd);
        let nl = Nl { fd: nl_fd, fam };
        let idx = get_device_idx(&nl);
        let mut dev = Device { nl, idx, polled_once: false };
        dev.bring_up();
        dev
    }

    fn bring_up(&mut self) {
        let _ = self.nl.cmd_simple(NFC_CMD_DEV_DOWN, self.idx);
        let mut up = false;
        for _ in 0..6 {
            if self.nl.cmd_simple(NFC_CMD_DEV_UP, self.idx) == 0 {
                up = true;
                break;
            }
            let _ = self.nl.cmd_simple(NFC_CMD_DEV_DOWN, self.idx);
            unsafe { usleep(700000) };
        }
        if !up {
            panic!("DEV_UP never succeeded");
        }
        self.start_poll_retry();
        // Sacrifice round: walk RF_DEACTIVATE once so a stale kernel
        // reassembly fragment from earlier sessions is freed.
        if let Some((sfd, _)) = self.connect() {
            unsafe { close(sfd) };
            unsafe { usleep(600000) };
        }
        self.repoll();
    }

    fn start_poll(&self) -> i32 {
        let mut b = msg_begin(self.nl.fam, NFC_CMD_START_POLL, NLM_F_ACK, 5);
        put_attr(&mut b, A_DEVICE_INDEX, &self.idx.to_ne_bytes());
        put_attr(&mut b, A_PROTOCOLS, &POLL_MASK.to_ne_bytes());
        finish_and_send(self.nl.fd, &mut b);
        self.nl.wait_ack()
    }

    /// Start polling, retrying while the RF state settles (a START
    /// issued while the previous discovery is still tearing down is
    /// rejected with EINVAL).
    fn start_poll_retry(&self) {
        for _ in 0..8 {
            if self.start_poll() == 0 {
                return;
            }
            unsafe { usleep(500000) };
        }
        panic!("START_POLL never succeeded");
    }


    /// After a target deactivates its index is dead; discovery must be
    /// restarted before the card can be connected again.
    pub fn repoll(&mut self) {
        let _ = self.nl.cmd_simple(NFC_CMD_STOP_POLL, self.idx);
        unsafe { usleep(150000) };
        self.start_poll_retry();
        unsafe { usleep(200000) };
        self.polled_once = true;
    }

    /// Connect to the freshest target. Retries because target indices
    /// roll forward and stale ones fail with EINVAL.
    pub fn connect(&self) -> Option<(i32, Target)> {
        for _ in 0..15 {
            let tg = match get_target(&self.nl, self.idx) {
                Some(t) => t,
                None => {
                    unsafe { usleep(300000) };
                    continue;
                }
            };
            let proto = [PROTO_MIFARE, PROTO_ISO14443, PROTO_JEWEL]
                .iter()
                .find(|&&p| tg.protocols & (1 << p) != 0)
                .copied()?;
            let s = unsafe { socket(AF_NFC, SOCK_SEQPACKET, 0) };
            if s < 0 {
                panic!("raw socket: {}", last_err());
            }
            let mut sa = [0u8; 16];
            sa[0..2].copy_from_slice(&(AF_NFC as u16).to_ne_bytes());
            sa[4..8].copy_from_slice(&self.idx.to_ne_bytes());
            sa[8..12].copy_from_slice(&tg.idx.to_ne_bytes());
            sa[12..16].copy_from_slice(&proto.to_ne_bytes());
            let rc = unsafe { connect(s, sa.as_ptr(), sa.len() as u32) };
            if rc == 0 {
                return Some((s, tg));
            }
            unsafe { close(s) };
            unsafe { usleep(200000) };
        }
        None
    }

    pub fn down(&self) {
        for _ in 0..6 {
            if self.nl.cmd_simple(NFC_CMD_DEV_DOWN, self.idx) == 0 {
                return;
            }
            unsafe { usleep(500000) };
        }
    }
}

#[derive(Clone, Copy)]
pub struct KeyCand {
    pub sel: u8,
    pub key: [u8; 6],
}

#[derive(Default, Clone, serde::Serialize, serde::Deserialize)]
pub struct SectorDump {
    pub key_sel: Option<u8>,
    pub key: Option<[u8; 6]>,
    pub blocks: [Option<[u8; 16]>; 4],
}

/// One live RF session: a raw-socket connection to the activated
/// target. A failed AUTH poisons it (the kernel answers further
/// exchanges with EPIPE by design), so it must be dropped and the
/// target reconnected before the next attempt. A successful AUTH
/// leaves it usable — the next sector can authenticate on the same
/// activation. That reuse is what makes a full-card read fast: the
/// original probe-era flow re-ran discovery + connect for every
/// single attempt (~2.8 s per sector, ~45 s per card).
struct Session {
    fd: i32,
    meta: (Vec<u8>, u16, u8), // nfcid1, sens_res, sel_res
}

/// Ensure a live session exists. The first open of a read rides on
/// bring_up's discovery; every later open must repoll first — after
/// a deactivation the target table keeps a stale entry whose index
/// connect() then rejects in a long retry loop (measured: minutes
/// lost). A fresh stop/start poll produces a fresh target at once.
fn session_open(dev: &mut Device, sess: &mut Option<Session>, repoll_first: bool) -> bool {
    if sess.is_some() {
        return true;
    }
    if repoll_first {
        dev.repoll();
    }
    for round in 0..2 {
        if let Some((fd, tg)) = dev.connect() {
            *sess = Some(Session {
                fd,
                meta: (tg.nfcid1.clone(), tg.sens_res, tg.sel_res),
            });
            return true;
        }
        if round == 0 {
            dev.repoll();
        }
    }
    false
}

fn session_drop(sess: &mut Option<Session>) {
    if let Some(s) = sess.take() {
        unsafe { close(s.fd) };
    }
}

/// Try to read one sector with one key candidate on the live
/// session. Returns true when all 4 blocks were read. On AUTH
/// failure the session is poisoned and dropped; on a mid-sector
/// read failure the session is dropped too and the most complete
/// partial dump is kept in `best`.
fn sector_try(sess: &mut Option<Session>, sector: u8, cand: &KeyCand, best: &mut SectorDump) -> bool {
    let fd = sess.as_ref().map(|s| s.fd).unwrap();
    let mut auth = vec![0x40u8, sector, cand.sel];
    auth.extend_from_slice(&cand.key);
    let ok = matches!(xchg(fd, &auth), Some(r) if !r.is_empty() && r[0] == 0x40);
    if !ok {
        session_drop(sess);
        return false;
    }
    let mut sd = SectorDump {
        key_sel: Some(cand.sel),
        key: Some(cand.key),
        blocks: [None; 4],
    };
    unsafe { usleep(50000) };
    for b in 0..4u8 {
        let blk = sector * 4 + b;
        match xchg(fd, &[0x10, 0x30, blk]) {
            Some(r) if r.len() >= 17 && r[0] == 0x10 => {
                let mut d = [0u8; 16];
                d.copy_from_slice(&r[1..17]);
                sd.blocks[b as usize] = Some(d);
            }
            _ => {
                if sd.blocks.iter().filter(|x| x.is_some()).count()
                    > best.blocks.iter().filter(|x| x.is_some()).count()
                {
                    *best = sd;
                }
                session_drop(sess);
                return false;
            }
        }
        unsafe { usleep(30000) };
    }
    *best = sd;
    true
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct CardDump {
    pub uid: Vec<u8>,
    pub atqa: u16,
    pub sak: u8,
    pub sectors: Vec<SectorDump>,
}

/// Read all 16 sectors of the card in range. Default candidates:
/// factory key as A, then as B (per sector). One RF session is kept
/// across sectors; per candidate there are up to two attempts — the
/// first may reuse the live session, the second always reconnects
/// first, so a stale session is not mistaken for a wrong key.
pub fn read_full_card(dev: &mut Device) -> Option<CardDump> {
    let cands = [
        KeyCand { sel: KEY_A, key: FACTORY_KEY },
        KeyCand { sel: KEY_B, key: FACTORY_KEY },
    ];
    let mut sectors: Vec<SectorDump> = Vec::new();
    let mut sess: Option<Session> = None;
    let mut opened_once = false;
    let mut meta: Option<(Vec<u8>, u16, u8)> = None;
    for s in 0..16u8 {
        let mut best = SectorDump::default();
        'cands: for cand in &cands {
            for attempt in 0..2 {
                if attempt == 1 {
                    session_drop(&mut sess);
                }
                if !session_open(dev, &mut sess, opened_once) {
                    break 'cands;
                }
                opened_once = true;
                if meta.is_none() {
                    meta = sess.as_ref().map(|x| x.meta.clone());
                }
                if sector_try(&mut sess, s, cand, &mut best) {
                    break 'cands;
                }
            }
        }
        sectors.push(best);
        if s == 0 && meta.is_none() {
            // No target was ever seen: there is no card in the field
            // (or discovery is wedged). Grinding through the other
            // 15 sectors would only repeat the same discovery
            // timeouts for minutes — fail fast instead.
            break;
        }
    }
    session_drop(&mut sess);
    meta.map(|(uid, atqa, sak)| CardDump {
        uid,
        atqa,
        sak,
        sectors,
    })
}

pub fn hexs(p: &[u8]) -> String {
    p.iter()
        .map(|b| format!("{:02X}", b))
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn asciis(p: &[u8]) -> String {
    p.iter()
        .map(|&b| {
            if (0x20..0x7f).contains(&b) {
                b as char
            } else {
                '.'
            }
        })
        .collect()
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn now_iso() -> String {
    // GNU date renders the local ISO timestamp, matching the
    // events.jsonl convention the python watcher used.
    match std::process::Command::new("date")
        .arg("+%Y-%m-%dT%H:%M:%S%:z")
        .output()
    {
        Ok(o) => String::from_utf8_lossy(&o.stdout).trim().to_string(),
        Err(_) => "unknown".to_string(),
    }
}

/// The read lock is the coordination point between the resident
/// watch and card reads: whoever reads (the GUI around its helper
/// call, or dump/dump-json directly) holds /tmp/nfctool-read.lock
/// for the duration, and watch stays off the adapter while a fresh
/// lock exists. The lock carries the reader's start time so one
/// orphaned by a crash goes stale (reads are bounded well under
/// this; the GUI's helper timeout is 60 s).
const READ_LOCK: &str = "/tmp/nfctool-read.lock";
const READ_LOCK_STALE_SECS: u64 = 120;

fn take_read_lock() {
    use std::io::Write as _;
    if let Ok(mut f) = std::fs::File::create(READ_LOCK) {
        let _ = f.write_all(now_secs().to_string().as_bytes());
    }
}

fn drop_read_lock() {
    let _ = std::fs::remove_file(READ_LOCK);
}

/// RAII form of the read lock: the file is removed on drop, so a
/// reader that panics mid-read (the engine's bring-up panics when
/// the device state fights it) cannot leave a lock behind for
/// the watch to mute itself against.
struct ReadLockGuard;

impl ReadLockGuard {
    fn new() -> ReadLockGuard {
        take_read_lock();
        ReadLockGuard
    }
}

impl Drop for ReadLockGuard {
    fn drop(&mut self) {
        drop_read_lock();
    }
}

/// After taking the read lock, give a running watch one full
/// burst cycle to notice the lock, release the device and go
/// quiet. Without this drain, a burst that started just before
/// the lock was taken can still be in flight — and its closing
/// STOP_POLL / the yield's DEV_DOWN then lands inside this
/// reader's bring-up and session, which the reader experiences
/// as a card that is present (the watch just found it) yet
/// impossible to read (seen live 2026-11-06: rc=3 with the card
/// on the phone the whole time).
fn drain_watch_burst() {
    std::thread::sleep(std::time::Duration::from_millis(1800));
}

fn read_lock_active() -> bool {
    match std::fs::read_to_string(READ_LOCK) {
        Ok(s) => {
            let started = s.trim().parse::<u64>().unwrap_or(0);
            now_secs().saturating_sub(started) < READ_LOCK_STALE_SECS
        }
        Err(_) => false,
    }
}

static WATCH_STOP: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

extern "C" fn watch_sig(_signum: i32) {
    WATCH_STOP.store(true, std::sync::atomic::Ordering::SeqCst);
}

fn watch_emit(obj: &serde_json::Value) {
    use std::io::Write as _;
    let line = obj.to_string();
    println!("{}", line);
    let _ = std::io::stdout().flush();
    // Best effort: keep appending to the same events file the
    // python watcher maintained, so the log history stays in
    // one place.
    let dir = "/home/user/nfc-log";
    if std::fs::create_dir_all(dir).is_ok() {
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(format!("{}/events.jsonl", dir))
        {
            let _ = writeln!(f, "{}", line);
        }
    }
}

/// Resident card-presence monitor (2026-11-06; replaces the
/// neard + python watcher pair, whose D-Bus handover proved too
/// brittle). Burst-polls the adapter on this binary's own genl
/// channel and logs one JSONL event per presentation: found
/// (UID / ATQA / SAK from the discovery attributes — no session,
/// no AUTH, so none of the session-poisoning failure modes) and
/// lost (after two consecutive empty bursts, to debounce field
/// edge flicker). Between bursts the poll is stopped, and while
/// a read lock exists the watch stays off the adapter entirely,
/// so deep reads never contend with it.
fn cmd_watch() {
    unsafe {
        signal(2, watch_sig as usize);
        signal(15, watch_sig as usize);
    }
    let fd = unsafe { socket(AF_NETLINK, SOCK_RAW, NETLINK_GENERIC) };
    if fd < 0 {
        panic!("netlink socket: {}", last_err());
    }
    let fam = resolve_family(fd);
    let nl = Nl { fd, fam };
    let idx = get_device_idx(&nl);
    let _ = nl.cmd_simple(NFC_CMD_DEV_DOWN, idx);
    let mut up = false;
    for _ in 0..6 {
        if nl.cmd_simple(NFC_CMD_DEV_UP, idx) == 0 {
            up = true;
            break;
        }
        let _ = nl.cmd_simple(NFC_CMD_DEV_DOWN, idx);
        unsafe { usleep(700000) };
    }
    if !up {
        panic!("DEV_UP never succeeded");
    }
    watch_emit(&serde_json::json!({
        "event": "watcher_start", "status": "ok", "ts": now_iso(),
    }));
    let mut present: Option<Target> = None;
    let mut misses = 0u32;
    let mut up = true;
    while !WATCH_STOP.load(std::sync::atomic::Ordering::SeqCst) {
        if read_lock_active() {
            // A read owns the adapter: release it COMPLETELY
            // (stop the poll and power the device down), not
            // just politely — the reader's bring-up cannot seize
            // a device that is merely up-but-idle (its DEV_DOWN
            // is refused while a poll is active and its DEV_UP
            // then fails EBUSY; seen live 2026-11-06 as a
            // 'DEV_UP never succeeded' panic). Also drop
            // presence state silently — the field picture may
            // change completely under the reader, so no lost
            // event is owed.
            if up {
                let _ = nl.cmd_simple(NFC_CMD_STOP_POLL, idx);
                let _ = nl.cmd_simple(NFC_CMD_DEV_DOWN, idx);
                up = false;
            }
            present = None;
            misses = 0;
            unsafe { usleep(300000) };
            continue;
        }
        if !up {
            // Re-acquire after a yield, or after a finished
            // read powered the device down on its way out.
            let _ = nl.cmd_simple(NFC_CMD_DEV_DOWN, idx);
            if nl.cmd_simple(NFC_CMD_DEV_UP, idx) == 0 {
                up = true;
            } else {
                unsafe { usleep(500000) };
                continue;
            }
        }
        // One discovery burst: start the poll, give targets a
        // moment to answer, snapshot the target list, stop.
        let mut b = msg_begin(nl.fam, NFC_CMD_START_POLL, NLM_F_ACK, 5);
        put_attr(&mut b, A_DEVICE_INDEX, &idx.to_ne_bytes());
        put_attr(&mut b, A_PROTOCOLS, &POLL_MASK.to_ne_bytes());
        finish_and_send(nl.fd, &mut b);
        if nl.wait_ack() != 0 {
            // Poll refused — the device state moved under us.
            // Mark it down; the next cycle re-acquires cleanly.
            up = false;
            unsafe { usleep(400000) };
            continue;
        }
        unsafe { usleep(700000) };
        let tg = get_target(&nl, idx);
        let _ = nl.cmd_simple(NFC_CMD_STOP_POLL, idx);
        match (tg, &present) {
            (Some(t), None) => {
                watch_emit(&serde_json::json!({
                    "event": "found",
                    "ts": now_iso(),
                    "uid": hexs(&t.nfcid1).replace(' ', ""),
                    "atqa": t.sens_res,
                    "sak": t.sel_res,
                }));
                present = Some(t);
                misses = 0;
            }
            (Some(t), Some(p)) => {
                if t.nfcid1 != p.nfcid1 {
                    // A different card swapped in without a gap.
                    watch_emit(&serde_json::json!({
                        "event": "lost", "ts": now_iso(),
                        "uid": hexs(&p.nfcid1).replace(' ', ""),
                    }));
                    watch_emit(&serde_json::json!({
                        "event": "found",
                        "ts": now_iso(),
                        "uid": hexs(&t.nfcid1).replace(' ', ""),
                        "atqa": t.sens_res,
                        "sak": t.sel_res,
                    }));
                    present = Some(t);
                }
                misses = 0;
            }
            (None, Some(p)) => {
                misses += 1;
                if misses >= 2 {
                    watch_emit(&serde_json::json!({
                        "event": "lost", "ts": now_iso(),
                        "uid": hexs(&p.nfcid1).replace(' ', ""),
                    }));
                    present = None;
                    misses = 0;
                }
            }
            (None, None) => {}
        }
        unsafe { usleep(600000) };
    }
    let _ = nl.cmd_simple(NFC_CMD_STOP_POLL, idx);
    let _ = nl.cmd_simple(NFC_CMD_DEV_DOWN, idx);
    watch_emit(&serde_json::json!({
        "event": "watcher_stop", "status": "ok", "ts": now_iso(),
    }));
}

/// Machine-readable dump for the GUI's privileged helper path: the GUI
/// runs unprivileged (DEV_UP is GENL admin-only and fails EPERM), so it
/// shells out to `sudo nfctool dump-json` and parses this JSON.
fn cmd_dump_json() {
    let _lock = ReadLockGuard::new();
    drain_watch_burst();
    let mut dev = Device::open();
    let dump = read_full_card(&mut dev);
    dev.down();
    match dump {
        Some(d) => println!("{}", serde_json::to_string(&d).unwrap()),
        None => std::process::exit(3),
    }
}

fn cmd_dump() {
    let _lock = ReadLockGuard::new();
    drain_watch_burst();
    let mut dev = Device::open();
    let dump = read_full_card(&mut dev);
    dev.down();
    let dump = match dump {
        Some(d) => d,
        None => {
            println!("no card seen");
            return;
        }
    };
    println!("UID: {}", hexs(&dump.uid));
    println!("ATQA: 0x{:04X}  SAK: 0x{:02X}", dump.atqa, dump.sak);
    for (i, sd) in dump.sectors.iter().enumerate() {
        match (sd.key_sel, sd.key) {
            (Some(sel), Some(k)) => println!(
                "sector {:2} key {} {}:",
                i,
                if sel == KEY_A { "A" } else { "B" },
                hexs(&k)
            ),
            _ => println!("sector {:2} LOCKED (no candidate key worked)", i),
        }
        for (b, blk) in sd.blocks.iter().enumerate() {
            match blk {
                Some(d) => println!("  blk {:2}: {}  |{}|", i * 4 + b, hexs(d), asciis(d)),
                None => println!("  blk {:2}: -- unreadable --", i * 4 + b),
            }
        }
    }
}

mod gui;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(|s| s.as_str()) {
        Some("dump") => cmd_dump(),
        Some("dump-json") => cmd_dump_json(),
        Some("watch") => cmd_watch(),
        Some("gui") => gui::run_gui(),
        _ => {
            eprintln!("usage: nfctool <dump|dump-json|watch|gui>  (dump and watch need root: DEV_UP is admin-only)");
            std::process::exit(2);
        }
    }
}
