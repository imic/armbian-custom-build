# armbian-custom-build（中文说明）

本仓库是 [armbian/build](https://github.com/armbian/build) 的个人 fork，用于为
**OnePlus 8T（kebab，SM8250）** 构建可直接刷入的 Armbian 镜像（boot + rootfs）。
与上游的全部差异只放在 `userpatches/` 一棵树里，框架本体文件不做改动，
上游更新时用 GitHub 的 **Sync fork** 同步即可。本仓库不向上游提交 PR。

本系统的全部定制与构建（内核补丁、设备树、镜像定制、本文档）
由 Muse（muse.ai）构建完成，**请谨慎使用**。

## 我们改了什么（userpatches/）

| 路径 | 内容 |
|---|---|
| `kernel/archive/sm8250-6.18/nfc-mifare-tagcmd.patch` | NFC：NCI 核心支持 NXP 私有 TAG-CMD 接口（0x80），使 SN100T 能读 MIFARE Classic 卡；所有路径按控制器/卡类型门控，对其它设备零行为变化 |
| `kernel/archive/sm8250-6.18/qcom-pon-reset-reason.patch` | PMIC PON：开机时把上一次断电/复位原因打进内核日志（GEN2），用于无日志硬重启的定性 |
| `kernel/archive/sm8250-6.18/kebab-device-tree.patch` | 设备树全量使能：NFC 引脚拆分、QCA6390 PMU 节点（BT 使能 tlmm21 由 PMU 电源时序器驱动；WiFi 使能 tlmm20 保留 GPIO hog——使能被 hog 常按时控制器无法真正断电重设、任何二次 setup 都会卡死其 UART，run #7 实测）、面板（AMB655X）+ GPU、USB3/OTG/Type-C PD、PON 的 bootloader/recovery 模式位、充电器与 VBUS、马达；并删去框架基线给 pcie0_phy 挂的 `power-domains = <&qca639x>`——本构建里无人注册该电源域，PHY 探测会死等它、QCA6390 永不枚举（run #6 实测无 WiFi 的根因） |
| `kernel/archive/sm8250-6.18/samsung-amb655x-panel.patch` | 面板驱动：主线与框架补丁集都没有 AMB655X（1080x2400 DSC）驱动，缺了它 MSM DRM 无法绑定、实体屏永不亮。此驱动为参考机 TEST 内核所带版本（sm8250-mainline 面板栈），已在本机面板实证；仅一处 API 适配（DSI 核心删掉的 write_long_multi 映射到 write_seq_multi，与现行主线三星 DSC 面板同法） |
| `config/kernel/linux-sm8250-current.config` | 内核配置：以框架版为底，RFKILL、POWER_RESET_QCOM_PON 改 =y，并内置 NFC 全套 =y（功能随 boot 镜像走，不依赖 rootfs 模块）；另钉 CONFIG_DRM_PANEL_SAMSUNG_AMB655X=y 启用上面的面板驱动；CONFIG_DEVFREQ_THERMAL=y 使 GPU 的 devfreq 散热设备能注册（缺它内核每次开机报 cooling 注册失败，2026-11-07 定位） |
| `overlay/` | 直接铺进 rootfs 的文件：systemd 服务（限充、nfctool NFC 常驻监控、蓝牙地址登记、WiFi 原厂地址 .link 生成、自动熄屏、音量键调亮度；WiFi 加入、旧蓝牙改址与旧 python NFC watcher 已退役、仅存档不启用）、脚本、NTP 配置、桌面启动器（熄屏/亮屏/NFC 工具，走 /etc/skel) |
| `customize-image.sh` | 构建收尾在 rootfs 内执行（先把 overlay 从构建框架挂载的 /tmp/overlay 复制进镜像——框架本身不代劳）：只装系统级包：bluez（蓝牙用户态）、gpiod、wpasupplicant、dhcpcd-base；polkitd 刻意不装（与原版镜像一致的包面裁定，2026-11-06；桌面日后装 polkit 代理包时 polkitd 会随其依赖自动回来）；桌面不进镜像——构建 chroot 装出的 Xfce 组成比参考机手动安装明显更重，2026-11-06 改为设备端脚本 /usr/local/bin/install-desktop.sh 在真机上 apt 安装（与手动安装同路，装后仍按需启动）；neard 在 Ubuntu resolute 源里没有，构建时从 Debian 池下载与参考机相同的 0.20-3 .deb 并校验哈希后本地安装，但禁用其自启（NFC 适配器默认无人占用，常驻监控由 nfctool watch 承担，2026-11-06 裁定）；另补装板级 hook 因 resolute 不在其支持名单而整批跳过的板级包（alsa-ucm-conf、qbootctl、qrtr-tools、mkbootimg、dropbear-bin，以及照板级 hook 做法临时启用 armbian 源装的 unudhcpd）并 enable qbootctl 与 usbgadget-rndis；enable 各服务，默认目标 multi-user，lightdm 若装了一律不自启（桌面按需 `systemctl start lightdm`，xrdp 不受影响）；overlay 的 /etc/skel 预置 Xfce 桌面图标默认（不显示磁盘/分区与可移动卷图标，本机约百个 GPT 分区否则全糊在桌面上）与终端配色（深色 terminalrc，避免 Ubuntu 原装 xfce4-terminal 的白底默认观感）；把镜像内 /boot/armbianEnv.txt（及 /usr/share/armbian/ 副本）的 abl_boot_partition_label 改写为 boot_a 使镜像按 A 槽打包（框架的 bootenv 文件从其自身 config/bootenv/ 直读、无 userpatches 覆盖路径，eed93414 曾放的 userpatches/config/bootenv/qcom-abl.txt 从未生效、run #7 仍打 _b 戳，已删除该文件；改写后校验该行，缺失即构建失败，杜绝静默打错戳）；复制后把 overlay 覆盖到的全部路径属主归正为 root（overlay 经构建链送达时带构建机 uid 1001，`cp -a` 会把 /etc、/usr 等既有目录染成该属主；timedated 的 unit 只有 CAP_SYS_TIME、无 CAP_DAC_OVERRIDE，root 身份也写不进 1001 属主的 /etc，真实时区变更会以 EACCES 失败——2026-11-10 定位并在机验证，归正后 timedatectl 恢复正常；无操作的目标时区不触发写入，曾长期掩盖此问题）；mask 掉 serial-getty@ttyS0（本板无此设备，通用镜像的该服务每次开机白等约 90 秒）；并把 BSP 包里的 DSP 固件按设备树要求的名字复制到 /lib/firmware/postmarketos/（adsp/cdsp/venus，与参考机文件哈希一致）；并按实测白名单修剪固件，只保留本机（SM8250 + QCA6390）能用到的约 105 MB，其余约 2.2 GB 在构建时删掉 |
| `tools/nfctool/` | NFC 读卡工具的 Rust 源码（只读，绝不写卡；含 watch 常驻监控模式：以 JSONL 记录刷卡行为，与读卡以 /tmp/nfctool-read.lock 协调让路）；由该源码以 `cargo build --release --target aarch64-unknown-linux-musl` 构建的二进制以 base64 形式存放在 `overlay/usr/local/bin/nfctool.b64`（仓库推送通道只收 UTF-8 文本；customize 在构建时解码、校验 sha256 42e7827a… 后落位 /usr/local/bin/nfctool，校验不过即构建失败），镜像自带、skel 有桌面启动器；卡库数据（~/.nfctool-cards.json）属个人数据，仍按刷后恢复清单处理 |

## 构建

- **GitHub Actions**：先在仓库 Actions 页启用 Actions（fork 默认关闭，仓主点一次），
  然后 Actions → “Build kebab image” → Run workflow，可选机型/发行版/整图或只编内核，（desktop 输入已失效：镜像恒为 minimal，桌面在设备上装）。
  产物由官方 armbian/build Action 生成并发布到 Releases。
  注意：workflow 文件 `.github/workflows/build-kebab.yml` 因 GitHub App 权限限制
  无法经 API 推送，需仓主在网页端手动新建一次（内容见仓库外随附说明），之后照常使用。
- **本地**：`./compile.sh build BOARD=oneplus-kebab BRANCH=current RELEASE=resolute BUILD_MINIMAL=yes KERNEL_CONFIGURE=no`
  （镜像恒为 minimal，不要加 BUILD_DESKTOP；桌面在设备上运行 install-desktop.sh 安装)。

首次构建出的镜像先不要急着刷：把产物 DTB 与内嵌 config 和参考机逐项对一遍
再决定刷机。刷机方法见下节。



## 分区布局（必读）

本机（256 GB 版）的分区表不是原厂布局。安装 Armbian 时重新分过区，
现状（参考机实测）：

- 固件类分区保持原厂 A/B 双份（xbl/abl/boot/dtbo/vbmeta 等），
  `boot_a`、`boot_b` 各 96 MiB；`super` 7 GiB（安卓动态分区，未动）。
- `userdata` 已从整机存储缩小到约 28 GiB，腾出的空间新建了两个分区：
  `esp`（477 MiB，当前未用）和 **`linux`（约 196 GiB）**。
- **Armbian 的 rootfs 实际装在 `linux` 分区**（启动后挂载为 /），
  不是 userdata。刷 rootfs 时目标是 `linux`，刷错到 userdata 不会生效。
- `persist`（32 MiB）存出厂校准数据（每台机器不同），fastboot 刷机
  不碰它，也不要手动去动它。
- 原厂状态的机器没有 `linux` 分区：要先按下节步骤重新分区，
  才能刷入本镜像；在原厂分区表上执行 `fastboot flash linux`
  只会报找不到分区。
- `dtbo_a` 是清空的（本镜像自带设备树，不用原厂 DTBO）；
  `dtbo_b` 保留原厂内容未动。
- Armbian 首启会把根文件系统扩展到分区大小（参考机 / 为 193 G）。


### 分区操作（原厂机首次安装时执行一次）

出处：amazingfate 的 Flashing Guide（同为 SM8250 平台的 elish 页，
机制相同）：https://github.com/amazingfate/armbian-xiaomi-elish/wiki/Flashing-Guide
以下坐标是参考机（256 GB）三次安装实测的定版数值；**分区号和起止点
因机型/容量而异，动手前必须先 `print` 核对自己机器的实际分区表，
不要照抄示例号码。**

1. 解锁 bootloader，进 TWRP（`fastboot boot twrp.img` 临时启动即可，
   不必刷入）。在 TWRP 的 Mount 页确认 Data 未挂载。
2. 在 TWRP 终端或 adb shell 里用 GNU parted 操作磁盘
   （TWRP 未自带时自行推入 parted 二进制）：

   ```
   parted /dev/block/sda
   (parted) unit GB
   (parted) print
   (parted) resizepart 28 41.9GB
   (parted) mkpart esp fat32 41.9GB 42.4GB
   (parted) set 29 esp on
   (parted) mkpart linux ext4 42.4GB 100%
   (parted) print
   (parted) quit
   ```

   要点：`resizepart` 会交互确认终点，照上面数值输入；结果应为
   28 号 userdata 30 GB、29 号 esp 500 MB、30 号 linux 吃满剩余空间。
   `mkpart` 只写分区表条目、不格式化：esp 至今保持未格式化、未使用
   （将来要用先 `mkfs.vfat`）；linux 分区的文件系统由刷入的 rootfs
   镜像自带。**1–27 号原厂分区和 persist 一律不碰。**
3. 回到 fastboot，按下一节刷机说明刷入 boot 与 rootfs；
   首次安装额外执行一次 `fastboot erase dtbo_a`（清掉 A 槽的
   原厂 DTBO，本镜像自带设备树）。

警告：分区操作不可逆，userdata 里的数据会全部丢失；坐标输错可能
变砖，救砖走后面的 MSM 一节（但布局要从本节重做）。

## 刷机说明

前提：bootloader 已解锁，电脑装有 fastboot。产物为两个文件：
boot 镜像（`*.boot_sm8250-oneplus-kebab.img`）和 rootfs 镜像（`*.rootfs.img`，
.xz 的先解压）。

> ⚠️ **只准用 A 槽位，B 槽位会死机。**
> 刷机前先确认当前活动槽位是 a；是 b 的先切回 a。不要把本镜像刷到 B 槽。
>
> 本仓库的镜像是 **A 槽打包**的：框架默认的 ABL 模板把镜像按 B 槽打包
> （`abl_boot_partition_label=boot_b`，cmdline 里会钉死 `slot_suffix=_b`，
> 系统内的 qbootctl 等工具据此记账）；本仓库由 customize-image.sh 在
> 构建时把镜像内 /boot/armbianEnv.txt 的该标签改写为 `boot_a`（校验
> 不过即构建失败），使打包槽位与实际安装槽位一致。注意：run #7 的
> 镜像仍是 _b 戳（当时的 userpatches bootenv 覆盖路线被证实无效，
> 见文件清单），刷 #7 后须照刷机步骤先在 fastboot 里 set-active=a。
> **不要拿上游默认模板构建的镜像照本节刷 A**——槽位记账错位正是
> 2026-11-06 热重启掉进 900E 的已证实根因（戳记是打包标签、并非
> 实际启动槽位，记账全记到了 B 头上，A 永不被标记成功）。run #8 起
> 戳记已改对：首启实测 qbootctl 显示 A 槽 Active=1、Successful=1
> （由系统自标），记账对齐已验证。

```bash
fastboot getvar current-slot        # 必须是 a
fastboot --set-active=a             # 不是 a 时执行，然后重启回 bootloader
fastboot flash boot_a <boot 镜像>
fastboot flash linux <rootfs 镜像>
fastboot reboot
```

首次启动会进入 Armbian 首启向导（建用户、改密码、配 WiFi、设时区）。
向导的时区步可正常保存（构建已含属主归正，见文件清单）。
向导的 WiFi 步有个已知边界：关联成功后它还要 curl 公网地址验证出口，
在路由器开启 fake-IP 代理（如 OpenClash fake-ip 模式）的网络里，
该探测从本机必然失败、向导会删掉刚写好的配置并报连接失败——
这不是密码或信号问题；跳过该步、登录后用 armbian-config 或手写
netplan 配置连接即可（配置在就会每次开机自动连）。
语言选择若只给出基于地理位置的短名单：在 "Set user language based
on your location? [Y/n]" 处答 **N**，名单会变成全量、时区改为手动选。
刷完 rootfs 后，每台设备自己的数据要本地恢复，见下一节。

只更新内核/boot 时（rootfs 不动）：只刷 `boot_a` 一步即可，
回滚就是把旧 boot 镜像刷回 `boot_a`。

## MSM 工具（EDL 救砖 / 回官方系统）

连 fastboot 都进不去时，用高通 EDL（9008）模式整机救砖。工具是
OnePlus 官方售后工具 **MsmDownloadTool V4.0**（仅 Windows），随各区域的
EDL 包附带，没有独立通用版；OnePlus 8T 的包由 XDA 的
Some_Random_Username 发布，按区域分（均为 Android 11 底包）：

- 国际版 KB05AA、欧洲版 KB05BA、印度版 KB05DA、T-Mobile 版 KB09CB
- XDA 原帖（T-Mobile 版）：https://forum.xda-developers.com/t/op8t-oos-tmo-kb09cb-unbrick-tool-to-restore-your-device-to-oxygenos.4180981/
- 各区域下载汇总：https://droidwin.com/download-msm-download-tool-unbrick-any-oneplus-device/
- 图文教程：https://www.thecustomdroid.com/oneplus-8t-unbrick-guide/

用法要点：**MSM 工具必须在断网状态下启动**（联网启动会校验失败）。
启动工具并选对应区域 → 让手机进入 9008（EDL）模式 → 点 Start，
约几分钟完成。进入 9008 的方式按手机当前状态选：

- **系统正常**：临时启动 TWRP（`fastboot boot twrp.img`），由 TWRP
  重启进入 EDL 模式。
- **关机状态**：同时按住音量+和音量-不放、插上 USB。
- **已卡在 900E**（USB 识别为 900E 诊断口）：按住电源和音量+，
  等串口（设备管理器里的 900E 设备）消失后**立即**改按音量-，
  抢在重新启动前进入 EDL。

成功进入时设备管理器应显示 Qualcomm HS-USB QDLoader 9008。

注意：

- MSM 会把整机刷回官方 OxygenOS 并**重新锁定 bootloader**，
  之后要重新解锁才能再刷本镜像。
- MSM 按官方包重写分区表，上面的自定义分区布局（linux/esp 分区）
  会一并消失；救砖后要从重新分区开始重装 Armbian。
- **不要碰 SMT Download 模式**，会抹掉 IMEI 和 Widevine 证书。
- 选包必须对准机器的区域版本，选错会在校验环节被拒。

## 每台设备各自的数据（不在本仓库）

镜像是通用的，不含任何个人/设备专属数据。下列内容在设备上本地生成，不入库：

- WiFi 凭据：首次运行向导里填写，写入 netplan 配置（WiFi 由 netplan
  接管；旧的 wifi-join/wpa.conf 路径已于 2026-11-06 退役）
- 蓝牙地址：bt-provision.service 在开机事务内（实测约 11 秒完成）经 mgmt 登记本机地址——控制器上电时只有芯片占位地址（00:00:00:00:5A:AD），内核将其留在未配置名单、bluetoothd 不认，登记后才转正可用。地址取本机出厂值：读 persist 分区里原厂 WiFi 地址（qca6390/wlan_mac.bin 的 Intf0MacAddress）减 1（本机无独立蓝牙出厂地址，此为本机约定，2026-11-08 在 persist 全量核查后定版）；persist 读不到时退回 /etc/machine-id 派生地址。脚本给 btmgmt 喂一个保持打开的 stdin（btmgmt 在服务默认的 /dev/null stdin 下永不返回，这是它此前在开机阶段屡屡超时的真因，2026-11-10 二分定位）；旧 bt-fixed-mac 已退役
- SSH 密钥、sudo 配置、NFC 卡库（~/.nfctool-cards.json）与刷卡事件
  日志（~/nfc-log/events.jsonl，由 nfctool watch 追加写入）：均为
  设备本地数据

## 已知问题

- 显示：DSI PHY 在探测期偶发 PLL 锁定失败（dmesg 有 `DSI PLL lock
  failed`），该次开机无引导画面/控制台，但后续 modeset 可恢复、桌面
  能正常点亮（run #6–#8 均见，机制未定位，待查）。屏亮不起来先查
  这行日志，不要先怀疑用户空间。
- WiFi 地址：ath11k 不读 persist 里的出厂地址、也不接受运行时改址，
  镜像以 wlan-factory-mac.service 在开机时按 persist 出厂值生成 udev
  .link，使接口在设备创建时即以本机出厂地址出生并固定
  名为 wlp1s0（已解决 2026-11-10）。注意全新刷机后的**第一启** .link
  尚不存在、仍是模板地址，第二启起才是出厂地址；路由器静态租约请
  按出厂地址钉。
- 传感器中枢（SLPI）固件刻意不出货：blob 本身可得，但主线内核没有
  它的 QDI 主机服务，固件每次启动都在首次 QDI 调用崩溃并被以约
  3.6 次/秒无限重启、拖垮整机（2026-11-08 实测），故构建时不放入
  镜像；主线补上 QDI 服务前不要尝试启用。ADSP 固件加载报 -22、
  本机音频未启用（声卡不注册），为另一独立问题。
- 充电限制（80/50 滞回）只能拦下一次充电会话，掐不断进行中的会话：
  长插电仍会充满到 100%。脚本在 overlay 里原样保留，缺陷在此登记。
- 马达（haptics）：DT 与寄存器面已验证全对，但播放时无震感，剩余未知在
  LRA 端子电气层，待示波器实测；DT 现状保留。
- USB 主机供电（VBUS）不出电：OTG 切到主机模式后 PMIC 升压级不输出，
  U 盘等设备不枚举（寄存器级实证，2026-11-06；上游同平台有相同报告、
  主线未修）。数据通路本身是好的：用自带供电的 USB Hub/扩展坞由坞供
  电即可绕过（组合未实测）。
- `reboot bootloader` 的 DT 模式位已就位，功能实测已于 2026-11-05 通过。

## 本地克隆注意

框架自带的 `.gitignore` 会忽略 `userpatches/`。网页/API 提交不受影响；
本地改 userpatches 后要用 `git add -f userpatches/` 才能纳入提交。
