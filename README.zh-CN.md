# armbian-custom-build（中文说明）

本仓库是 [armbian/build](https://github.com/armbian/build) 的个人 fork，用于为
**OnePlus 8T（kebab，SM8250）** 构建可直接刷入的 Armbian 镜像（boot + rootfs）。
与上游的全部差异只放在 `userpatches/` 一棵树里，框架本体文件不做改动，
上游更新时用 GitHub 的 **Sync fork** 同步即可。本仓库不向上游提交 PR。

本系统的全部定制与构建（内核补丁、设备树、镜像定制、本文档）
由 Muse（muse.ai）完成。

## 我们改了什么（userpatches/）

| 路径 | 内容 |
|---|---|
| `kernel/sm8250-current/nfc-mifare-tagcmd.patch` | NFC：NCI 核心支持 NXP 私有 TAG-CMD 接口（0x80），使 SN100T 能读 MIFARE Classic 卡；所有路径按控制器/卡类型门控，对其它设备零行为变化 |
| `kernel/sm8250-current/qcom-pon-reset-reason.patch` | PMIC PON：开机时把上一次断电/复位原因打进内核日志（GEN2），用于无日志硬重启的定性 |
| `kernel/sm8250-current/kebab-device-tree.patch` | 设备树全量使能：NFC 引脚拆分、QCA6390 PMU 节点与 WiFi/BT 使能 GPIO hog、面板（AMB655X）+ GPU、USB3/OTG/Type-C PD、PON 的 bootloader/recovery 模式位、充电器与 VBUS、马达 |
| `config/kernel/linux-sm8250-current.config` | 内核配置：以框架版为底，RFKILL、POWER_RESET_QCOM_PON 改 =y，并内置 NFC 全套 =y（功能随 boot 镜像走，不依赖 rootfs 模块） |
| `overlay/` | 直接铺进 rootfs 的文件：systemd 服务（限充、NFC 监控、自动熄屏、音量键调亮度、WiFi 加入、QCA6390 使能、蓝牙地址）、脚本、NTP 配置、桌面启动器（熄屏/亮屏/NFC 工具，走 /etc/skel) |
| `customize-image.sh` | 构建收尾在 rootfs 内执行：安装桌面栈（xfce4/lightdm/xrdp，与参考机同款）、neard/gpiod 等包，enable 各服务，默认目标 multi-user；并把 BSP 包里的 DSP 固件按设备树要求的名字复制到 /lib/firmware/postmarketos/（adsp/cdsp/venus，与参考机文件哈希一致）；并按实测白名单修剪固件，只保留本机（SM8250 + QCA6390）能用到的约 105 MB，其余约 2.2 GB 在构建时删掉 |
| `tools/nfctool/` | NFC 读卡工具的 Rust 源码（只读，绝不写卡）；源码在此；二进制需在本机用 `cargo build --release --target aarch64-unknown-linux-musl` 构建后放进 `overlay/usr/local/bin/`（参考机现行二进制即如此构建） |

## 构建

- **GitHub Actions**：先在仓库 Actions 页启用 Actions（fork 默认关闭，仓主点一次），
  然后 Actions → “Build kebab image” → Run workflow，可选机型/发行版/整图或只编内核。
  产物由官方 armbian/build Action 生成并发布到 Releases。
  注意：workflow 文件 `.github/workflows/build-kebab.yml` 因 GitHub App 权限限制
  无法经 API 推送，需仓主在网页端手动新建一次（内容见仓库外随附说明），之后照常使用。
- **本地**：`./compile.sh build BOARD=oneplus-kebab BRANCH=current RELEASE=resolute BUILD_MINIMAL=yes KERNEL_CONFIGURE=no`
  （桌面由 customize-image.sh 安装，不要加 BUILD_DESKTOP)。

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

```bash
fastboot getvar current-slot        # 必须是 a
fastboot --set-active=a             # 不是 a 时执行，然后重启回 bootloader
fastboot flash boot_a <boot 镜像>
fastboot flash linux <rootfs 镜像>
fastboot reboot
```

首次启动会进入 Armbian 首启向导（建用户、改密码）。刷完 rootfs 后，
每台设备自己的数据要本地恢复，见下一节。

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

用法要点：启动工具并选对应区域 → 手机关机，同时按住音量+和音量-
不放、插上 USB（设备管理器应显示 Qualcomm HS-USB QDLoader 9008）→
点 Start，约几分钟完成。

注意：

- MSM 会把整机刷回官方 OxygenOS 并**重新锁定 bootloader**，
  之后要重新解锁才能再刷本镜像。
- MSM 按官方包重写分区表，上面的自定义分区布局（linux/esp 分区）
  会一并消失；救砖后要从重新分区开始重装 Armbian。
- **不要碰 SMT Download 模式**，会抹掉 IMEI 和 Widevine 证书。
- 选包必须对准机器的区域版本，选错会在校验环节被拒。

## 每台设备各自的数据（不在本仓库）

镜像是通用的，不含任何个人/设备专属数据。下列内容在设备上本地生成，不入库：

- WiFi 凭据：设备上的 `/root/wpa.conf`（wifi-join.sh 读取）
- WiFi MAC 钉定（可选）：`/etc/wifi-join.conf` 里写 `WIFI_MAC=...`；不写则不钉
- 蓝牙地址：由 machine-id 派生（bt-fixed-mac.sh)，不写死在设备树
- SSH 密钥、sudo 配置、NFC 卡库（~/.nfctool-cards.json)：均为设备本地数据

## 已知问题

- 充电限制（80/50 滞回）只能拦下一次充电会话，掐不断进行中的会话：
  长插电仍会充满到 100%。脚本在 overlay 里原样保留，缺陷在此登记。
- 马达（haptics）：DT 与寄存器面已验证全对，但播放时无震感，剩余未知在
  LRA 端子电气层，待示波器实测；DT 现状保留。
- `reboot bootloader` 的 DT 模式位已就位，功能实测尚未做。

## 本地克隆注意

框架自带的 `.gitignore` 会忽略 `userpatches/`。网页/API 提交不受影响；
本地改 userpatches 后要用 `git add -f userpatches/` 才能纳入提交。
