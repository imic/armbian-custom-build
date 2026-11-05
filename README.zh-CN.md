# armbian-custom-build（中文说明）

本仓库是 [armbian/build](https://github.com/armbian/build) 的个人 fork，用于为
**OnePlus 8T（kebab，SM8250）** 构建可直接刷入的 Armbian 镜像（boot + rootfs）。
与上游的全部差异只放在 `userpatches/` 一棵树里，框架本体文件不做改动，
上游更新时用 GitHub 的 **Sync fork** 同步即可。本仓库不向上游提交 PR。

## 我们改了什么（userpatches/）

| 路径 | 内容 |
|---|---|
| `kernel/sm8250-current/nfc-mifare-tagcmd.patch` | NFC：NCI 核心支持 NXP 私有 TAG-CMD 接口（0x80），使 SN100T 能读 MIFARE Classic 卡；所有路径按控制器/卡类型门控，对其它设备零行为变化 |
| `kernel/sm8250-current/qcom-pon-reset-reason.patch` | PMIC PON：开机时把上一次断电/复位原因打进内核日志（GEN2），用于无日志硬重启的定性 |
| `kernel/sm8250-current/kebab-device-tree.patch` | 设备树全量使能：NFC 引脚拆分、QCA6390 PMU 节点与 WiFi/BT 使能 GPIO hog、面板（AMB655X）+ GPU、USB3/OTG/Type-C PD、PON 的 bootloader/recovery 模式位、充电器与 VBUS、马达 |
| `config/kernel/linux-sm8250-current.config` | 内核配置：以框架版为底，RFKILL、POWER_RESET_QCOM_PON 改 =y，并内置 NFC 全套 =y（功能随 boot 镜像走，不依赖 rootfs 模块） |
| `overlay/` | 直接铺进 rootfs 的文件：systemd 服务（限充、NFC 监控、自动熄屏、音量键调亮度、WiFi 加入、QCA6390 使能、蓝牙地址）、脚本、NTP 配置、桌面启动器（熄屏/亮屏/NFC 工具，走 /etc/skel) |
| `customize-image.sh` | 构建收尾在 rootfs 内执行：安装桌面栈（xfce4/lightdm/xrdp，与参考机同款）、neard/gpiod 等包，enable 各服务，默认目标 multi-user；并把 BSP 包里的 DSP 固件按设备树要求的名字复制到 /lib/firmware/postmarketos/（adsp/cdsp/venus，与参考机文件哈希一致） |
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
再决定刷机。

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
