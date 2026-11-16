# 对抗性 review：`docs/design/architecture.md` + `docs/design/README.md`

## 0. 先回答被点名的六个问题（结论 + 证据）

| 问题 | 结论 |
|---|---|
| sysusers 账号 + `SupplementaryGroups=video` 与 `DynamicUser` 冲突 | **文档里没有 `DynamicUser`，所以当前不存在这个冲突**。systemd 245 的 `systemd.exec` 源文档里没有"二者不可组合"的限制；systemd 255 上 `systemd-analyze verify` 对 `DynamicUser=yes` + `SupplementaryGroups=video` 返回 0（接受）。上游 issue #8219 的坏情况是"动态组"进补充组，`video` 是静态组。真正该否决 `DynamicUser` 的理由是 UID 回收 + `/userdata/rec`、`/var/lib/tspi-cam` 的持久属主 + 隐含 `ProtectSystem=strict`（man 原话：UID/GID 会被回收，残留文件可能被别的 unit 拿到）。**但 `architecture.md:67` 写的"加入 video 组"机制不成立**：`sysusers.d` 的 `u` 行只创建同名主组，加入补充组必须另有 `m tspi-cam video` 行（man sysusers.d 已核对）。见 S1。 |
| socket `fchown` 0640/0660 与 umask/继承 | **模式不是问题，属主是硬伤**。`bind()` 后 inode 是 `0777 & ~umask`（systemd 默认 0022 → 0755），组已有 rw，`chmod 0660` 只是收紧 other 位；`connect()` 需要**写**权限（unix(7)），所以 0640 会让组内客户端连不上，0660 才是对的。但 `fchown(0, video_gid)` 在非 root 下必失败：WSL 实测 uid 1000 对自建 socket 调 `chown(...,0,gid)` → `EPERM`。而 `actiond` 按 `architecture.md:87` 是 `tspi-cam`，`RuntimeDirectory=` 建的目录属主就是 `tspi-cam:tspi-cam`（man：目录按 `User=/Group=` chown，stop 时删）。**`architecture.md:56-57/83/105-106` 与 `ipc.md:224` 的 `root:video` 契约在这个架构下不可能实现。** |
| udev 匹配写法 | `mpp_service`、`renderD*`、`card[0-9]`、`spidev3.0` 的名字都对（H5/H12 佐证）。错的是**集合**：漏了 `/dev/rga`（§4 自己说预览缩放走 RGA，研究文档明确要求同一条规则覆盖 `/dev/rga`）、`/dev/media0`（H14 用 `media-ctl` 配 rkisp 管线）、以及待确认的 `/dev/dma_heap/*`；另外 `spidev3.0` 开机**不存在**（H5：driver_override 不持久），so 规则再对也没节点可套。 |
| `current` 切换与 `ExecStart` | 路径本身可行（systemd 每次启动重新解析符号链接，`restart` 取新版本）。没定义的是：**怎么原子切**（`ln -sfn` 是 unlink+symlink，有 ENOENT 窗口；要 `mv -T` rename）、**旧版本保留几个**、**正在录制时切换/重启的收尾**（`TimeoutStopSec` 与 index 落盘），以及 unit 文件在 `/etc/systemd/system` 不在 release 里 → 回滚不原子。另外仓库现状（`deploy/systemd/actiond.service:8` 的 `/usr/local/bin/actiond`、`deploy/install.sh:10-13`）与 `architecture.md:71` 的"只使用 releases"直接冲突，且迁移未写进任何页面。 |
| `uid` 需要 `input` 组 | **自相矛盾**。`architecture.md:14/36` 说触摸是 `evdev → uid`，`:103` 又写"或走 Xorg 的 pointer 事件"。ui-stack 决策与实测（`ui-stack-decision.md:16`）是 Xorg/libinput 已把触摸当 pointer；Xorg 打开输入设备时会 grab，`uid` 再读 evdev 收不到事件。二选一没定，直接决定 `uid` 的依赖（evdev crate）、组（input）和 Xorg 配置。 |
| "一个机制一页" | **已经破了**。README:8 给 architecture 的"启动/部署顺序"与 README:16 给 deploy.md 的"安装布局、systemd 单元、组权限/udev、回滚"是同一批事实；`architecture.md:61-75、81-106` 全写了。另有三处重叠（gates、stride、socket 权限）。按 README:3-4 自己的规则，现在冻结就立刻自违。 |

---

## 1. Blocking（冻结前必须改）

### B1. socket 属主契约 `root:video` 与非 root 的 `actiond` 不可能同时成立 — P1
**位置**：`architecture.md:56-57`、`:83`、`:105-106`；`ipc.md:224`。
**错在哪**：`0660 root:video` + "bind 后 `fchown`" 的实际实现者是 `actiond`，而它在 `:87` 被规定为 `tspi-cam`。非 root 进程不能把属主改成 uid 0（实测 `EPERM`，chown(2) 也只允许 `CAP_CHOWN` 改属主）。`RuntimeDirectory=tspi-cam` 又保证 `/run/tspi-cam` 属主是 `tspi-cam`。结果是：要么 daemon 启动即 EPERM 退出，要么悄悄不设属主、客户端按 `video` 去连却失败。
**建议改法**：二选一并写死在两页：
- 首选 socket activation：`actiond.socket` 里 `ListenStream=/run/tspi-cam/record.sock`、`SocketMode=0660`、`SocketUser=root`、`SocketGroup=video`，`actiond` 只 `accept`（systemd 245 支持 `SocketUser/SocketGroup`），daemon 不再 `fchown/chmod`；
- 或保留 daemon 自建，但契约改成 `tspi-cam:video`：`fchown(fd, -1, video_gid)`（成员可 chgrp）后 `chmod 0660`，并把表格、mermaid、ipc.md §7 的 `root:video` 全部改掉。
删除"不靠 umask"之外的误导性描述：0640 不能给客户端用（`connect` 要写权限）。
**证明测试**：`systemctl start actiond; stat -c '%a %U:%G' /run/tspi-cam/*.sock` 必须与契约逐字节一致；`sudo -u k /opt/tspi-cam/current/bin/actionctl hello` 必须成功；若保留 daemon 自建，启动日志不得出现 `EPERM`。

### B2. `uid` 的运行账号 `k` 没有任何地方被加入 `video` — P1
**位置**：`architecture.md:103`（`uid` 以 `k` 运行、"通过 socket 组权限访问"）、`:56-57`（0660 组 `video`）。
**错在哪**：权限表把访问权交给 `video` 组，但全文没有任何机制把 `k` 放进 `video`。同一缺口也打掉 `/dev/dri/card0`（H10 的 `gbm_create_device(/dev/dri/card0)`）——Xorg 若以 `k` 跑也打不开。`actionctl` 在板上以 `k` 跑时同样连不上。
**建议改法**：在部署布局里明确一条（属 deploy.md，但 architecture 必须给出"谁在哪个组"的结论）：安装器执行 `usermod -aG video k`（或 sysusers `m k video`），并写明 `uid`/`actionctl` **必须重启登录会话**才拿到新组；同时把 `actiond` 的 `video` 来源写成 `SupplementaryGroups=video`（见 S1）。
**证明测试**：`id k` 含 `video`；`sudo -u k test -w /run/tspi-cam/record.sock`；`sudo -u k python3 -c 'import os;os.open("/dev/dri/card0",os.O_RDWR)'`（Xorg 用户是 `k` 时）。

### B3. 触摸归属自相矛盾，且 Xorg 根本不在服务划分/启动顺序里 — P1
**位置**：`architecture.md:14`（`touchscreen ── evdev ──▶ uid`）、`:36`（"evdev 触摸"）、`:103`（"或走 Xorg"）、`:121`（显示走 Xorg）。
**错在哪**：(a) evdev 与 Xorg-pointer 是两套互斥机制，Xorg/libinput 会 grab 设备，第二个读者收不到事件；二选一未定，直接影响 `uid` 的依赖与 `input` 组。(b) 选定的 UI 栈（`ui-stack-decision.md`：libmali 只有 x11 变体，无 X 时 EGL 不可用）必须有 Xorg，但原文的"两个守护进程"与服务表里没有 Xorg 的位置、用户、启动顺序和 `uid` 对 `DISPLAY` 的依赖——`uid.service` 起来时没有 `DISPLAY` 是必然的。
**建议改法**：服务表加一行 `xorg`（或明确它属于 `uid.service` 的子进程），写清：由谁启动（独立 unit 还是 uid 的 `ExecStartPre`）、以哪个用户跑（`k` 则需 `video`+`input`；root 则要说明 `-ac` 的信任边界）、`uid.service` 对它的 `After=`/`Requires=`，以及 `DISPLAY=:0` 由谁提供。触摸只保留 Xorg-pointer 一条路线，删除 evdev 箭头或标成"仅在无 X 的回退方案"。
**证明测试**：装完后 `systemctl list-units | grep -i xorg` 有明确 owner；`sudo -u k DISPLAY=:0 xdpyinfo >/dev/null` 成功顺序上早于 `uid` 的第一次 EGL 初始化；`fuser -v /dev/input/event*` 只显示 Xorg（或只显示 uid），不出现两个读者。

### B4. udev 授权集不完整：预览用的 `/dev/rga`、相机管线的 `/dev/media0` 都没有规则 — P1
**位置**：`architecture.md:88-89`（"设备访问**全部**由 … 授权"）、`:91-99`（规则块）、`:114`（预览由 RGA 缩放）、`:35`（actiond 的依赖列表）。
**错在哪**：规则块只有 video4linux / mpp_service / spi / drm。`docs/research/mpp-hw-encode.md:216` 明确写着"同一条 udev 规则覆盖 `/dev/rga`"，H14 的相机管线核对用的是 `media-ctl`（`/dev/media0`）。以 `tspi-cam` 运行时，`open("/dev/rga")`、`open("/dev/media0")` 会 EACCES——前者发生在第一次预览缩放，后者发生在 rkisp 配管线（可能在 `actiond` 启动阶段）。
**建议改法**：规则块补 `KERNEL=="rga", GROUP="video", MODE="0660"`、`SUBSYSTEM=="media", GROUP="video", MODE="0660"`，并把 `SUBSYSTEM=="dma_heap"`（或本板实际使用的 `/dev/ion`）列成"必须在板上确认"的一条；注释里不要再写"全部授权"。
**证明测试**：`for d in /dev/mpp_service /dev/rga /dev/media0; do sudo -u tspi-cam test -r "$d" && echo ok $d || echo FAIL $d; done`；再加一条以 `tspi-cam` 身份跑的 1 帧编码+缩放冒烟（`gst-launch-1.0 videotestsrc num-buffers=1 ! mpph264enc ! fakesink`）。

### B5. `spidev3.0` 开机不存在，规则没错但 IMU 仍然起不来；布局里也没有 `spidev-bind.service` — P1
**位置**：`architecture.md:88-89`（"当前 `/dev/spidev3.0` 是 `root:root 0600` … 补规则即可"）、`:93`（spidev 规则）、`:63-69`（安装布局只列 actiond/uid）、`:35`（actiond 依赖 `/dev/spidev3.0`）。
**错在哪**：`docs/architecture.md:740` H5 已实测：**重启后 `/dev/spidev3.0` 消失**，driver_override 绑定不持久，缓解方案是 `spidev-bind.service` 开机重绑，长期方案是改 DTS。新文档直接假设节点存在，把问题降级成权限问题，等于把已知失败藏起来；布局又没给这个 unit 留位置，实现阶段会按文档只写一条 udev 规则，然后在板上发现 IMU 永远打不开。
**建议改法**：布局加 `/etc/systemd/system/spidev-bind.service`（root，`Before=actiond.service`，绑定后由 udev 规则接管权限），`actiond.service` 用 `After=` 该 unit；文档写明"节点不存在时 `actiond` 报 `E_HW` 且 `record.status.last_error` 可见，不重试到超时"。
**证明测试**：重启后 `ls -l /dev/spidev3.0` 存在且为 `tspi-cam:video 0660`；`sudo -u tspi-cam python3 -c 'open("/dev/spidev3.0","r+b")'` 成功；`systemctl show -p After actiond` 含 `spidev-bind.service`。

### B6. 配置文件名 `recordd.toml` 是 microduck 残留，与仓库/服务名不一致 — P1
**位置**：`architecture.md:52-53`（`/etc/tspi-cam/recordd.toml`、`/var/lib/tspi-cam/recordd.toml`）。
**错在哪**：服务叫 `actiond`，仓库里是 `config/actiond.toml`，`deploy/install.sh:19-23` 与 `deploy/systemd/actiond.service:8` 都用 `actiond.toml`。`recordd` 只在这里出现。冻结后实现者会把这个字符串编进代码、安装器和 ipc 文档，两条路径只能二选一——这正是"现在跑的是哪一版/哪份配置"变猜谜的那类问题。
**建议改法**：两行统一改成 `actiond.toml`（若坚持改名，则同时改 install.sh、unit、`config/` 与全文引用，并写进 CHANGELOG）。
**证明测试**：`strings build/bin/actiond | grep -E 'recordd|actiond\.toml'` 只出现 `actiond.toml`；升级后 `ls /etc/tspi-cam` 与文档逐字一致。

### B7. 状态表把"用户层配置"的直接写权限给了 `uid`/`actionctl`，与 IPC 的 `device.config.set` 冲突，且 `/var/lib/tspi-cam` 属主未定 — P1
**位置**：`architecture.md:53`（`actionctl configure / uid` 写 `/var/lib/tspi-cam/*.toml`）、`:54`（`actioncal` 写 `calibration.json`）、`:55`（`actiond` 写 `device.json`）；对照 `ipc.md:156`（`device.config.set` 由 `actiond` 只写用户层）。
**错在哪**：两个写者、两种路径（直写文件 vs 走 IPC）。直写会绕过 `actiond` 在内存里的合并结果与校验，且 `uid` 以 `k` 运行、`/var/lib/tspi-cam` 按 systemd 约定属于 `tspi-cam`（`StateDirectory=`），`k` 根本写不进去；`actioncal` 以 `k` 跑同样写不了 `calibration.json`。目录属主/模式全文没写，实现只能猜。
**建议改法**：表格"谁写"改成 `actiond`（经 `device.config.set`），删掉 `uid` 直写；明文规定 `/var/lib/tspi-cam` 由 `StateDirectory=tspi-cam` 创建（`tspi-cam:tspi-cam 0755`），并给 `actioncal` 一个明确的落盘路径（跑成 `tspi-cam` 用户，或经一个 `device.calibration.save` 类调用由 `actiond` 落盘）；`calibration.json` 的 0644/0600 也要定。
**证明测试**：`stat -c '%U:%G %a' /var/lib/tspi-cam`；`sudo -u k sh -c 'echo x >> /var/lib/tspi-cam/actiond.toml'` 失败（证明"k 直写"方案不可实现）；`actionctl device.config.set …` 后 `stat` 的 mtime/owner 由 `actiond` 改写。

### B8. `device.lock` 随 `RuntimeDirectory` 在 stop 时被删，`actioncal` 的账号又未定义 → 标定流程起不来 — P1
**位置**：`architecture.md:58`（锁在 `/run/tspi-cam/`，"文件残留不代表被占用"）、`:38`（`actioncal` "只连 actiond 的 device.lock"）、`:105`（`RuntimeDirectory` 负责创建与清理）。
**错在哪**：man 明确 `RuntimeDirectory=` 的 innermost 目录在 unit 停止时删除。于是"`systemctl stop actiond` 后跑 `actioncal`"这条流程里锁文件已不存在；`/run` 是 root:root 0755，`k` 既不能建 `/run/tspi-cam` 也不能建锁文件 → `actioncal` 以 `k` 跑必然 EACCES。若改成 `sudo actioncal`，则"非 root"原则对标定工具失效，且锁的属主/权限故事又变了——文档两个都没写。另：锁文件无法"连接"（flock 是 open+flock），措辞也错。
**建议改法**：锁移出 RuntimeDirectory（如 `/var/lib/tspi-cam/device.lock`，`tspi-cam:video 0660`，用 tmpfiles 建），或明确 `actioncal` 以 `tspi-cam` 运行（`sudo -u tspi-cam`）并写进标定流程；给出 "actiond 停 → 锁仍存在 → actioncal 打开/加锁 → 退出后 actiond 重取" 的状态表。
**证明测试**：`systemctl stop actiond; ls -l /var/lib/tspi-cam/device.lock` 仍存在且 `sudo -u k`（或 `tspi-cam`）能 `flock -n`；`systemctl start actiond` 时若锁被占则拒绝开硬件并在 `last_error` 里给出 `E_CALIB`。

### B9. README 的归属规则在当前内容下立即自违（architecture 抢了 deploy.md 的整块内容）— P1
**位置**：`README.md:8`（architecture 拥有"启动/部署顺序"）vs `README.md:16`（deploy.md 拥有"安装布局、systemd 单元、组权限/udev、回滚"）；实际重复在 `architecture.md:61-75`（安装布局、releases/current、回滚理由）与 `:81-106`（mermaid 的 unit 依赖、sysusers/udev、RuntimeDirectory、fchown）。另外三处：`README.md:17` 说 gates.md 拥有 xtask 门禁，但 `ipc.md:228` 与 `interfaces.md:242-248` 各写了一份；`README.md:13` 把"步幅对齐"给 capture-encode.md，而 `interfaces.md:139-152` 才是 stride 定义（`ipc.md:177` 也指向它）；socket 权限同时出现在 `architecture.md:56-57`、`ipc.md:217-226`、以及 deploy.md 声明的"组权限"。
**错在哪**：README:3-4 自己规定"两页冲突时，不拥有该机制的那页是 bug"。现在 architecture 同时是 owner（README:8）和非 owner（README:16），deploy.md 还没写就已经被抢空；冻结后任何一处改动都会让另一页过期，这正是该索引要防的事。
**建议改法**：二选一——(a) architecture 只保留"运行时顺序与状态归属"，把安装布局、unit 内容、sysusers/udev、回滚步骤整体移到 deploy.md，正文改成一句链接；(b) 改 README:16，让 deploy.md 只拥有"交叉编译与打包"，architecture 明确拥有"安装布局/unit/udev"。gates、stride、socket 权限各指定唯一 owner，其余页只留链接（建议：gates → gates.md；stride → interfaces.md §3.2；socket 模式/属主 → ipc.md §7，architecture 只写"两个 socket 及其客户端"）。
**证明测试**：一个 grep 级别的文档测试（`xtask docs`）：对每个"机制关键词"（`SocketMode|0660|sysusers|udev|releases/current|steval`）断言只在 owner 页出现；现在跑会红，改完才绿。这也是 README 规则唯一可执行的形态。

---

## 2. Should fix

### S1. `sysusers` 只用 `u` 行加不进 `video`；同时把"不用 DynamicUser"写死 — P2
**位置**：`architecture.md:67`。`sysusers.d` 的 `u` 行只创建同名主组，加入补充组必须 `m tspi-cam video`；而 `architecture.md:87` 又说"加入 video 组"却没给机制。另：`DynamicUser` 当前没出现，但一个"无状态 Rust daemon"很容易被实现者写成 `DynamicUser=yes`，那时 `SupplementaryGroups=video` 虽被 systemd 接受（WSL 255 实测 verify 通过；245 源文档无禁止条款），但 `/userdata/rec`、`/var/lib/tspi-cam/device.json` 的持久属主会随 UID 回收而错乱，且隐含 `ProtectSystem=strict` 需要额外 `ReadWritePaths=`。
**改法**：sysusers 文件写两行（`u tspi-cam - "tspi-cam service" /var/lib/tspi-cam` + `m tspi-cam video`），unit 里用 `User=tspi-cam` + `SupplementaryGroups=video`，二者只留一个作为权威（另一个删掉），并加一句"**禁止 `DynamicUser=`**：持久状态目录与 `/userdata` 产物依赖稳定 UID"。
**测试**：`getent group video | grep tspi-cam`；`systemctl show actiond -p SupplementaryGroups`；把 unit 改成 `DynamicUser=yes` 时文档/门禁应报错（或在 deploy.md 里写明该负向验证）。

### S2. udev 规则前缀 70 与"防止镜像改动"的意图相反，且规则文件不在仓库 — P2
**位置**：`architecture.md:68`（`/etc/udev/rules.d/70-tspi-cam.rules`）、`:88`（`deploy/udev/70-tspi-cam.rules`，实际不存在——`deploy/` 只有 `build.sh`、`install.sh`、`systemd/actiond.service`）。
**错在哪**：udev 按文件名字典序读取，同名 key 后者覆盖；板级镜像若有 `99-*` 规则（很常见），70 会被覆盖，"防止镜像改动"不成立。且文档声称"仓库内"的规则文件不存在，冻结后安装器无从安装。
**改法**：改名 `99-tspi-cam.rules`（或 `90-`），在仓库补文件并把 `udevadm control --reload && udevadm trigger --action=change && udevadm settle` 写成安装步骤；文档里给出"装完必须 `udevadm test /sys/...` 验证 group"。
**测试**：`udevadm test /sys/class/video4linux/video0 2>&1 | grep GROUP` 输出 `video`；在 `/etc/udev/rules.d/99-vendor.rules` 里塞一条 `MODE="0600"` 反向验证它确实会赢（说明为什么必须用 99）。

### S3. `current` 的"原子切换"没有实现方法，回滚流程也没定义 — P2
**位置**：`architecture.md:65`（"原子切换，restart 生效"）、`:71-73`。
**错在哪**：`ln -sfn` 是 unlink+symlink，不是原子替换；切换瞬间 `ExecStart` 可能 ENOENT。unit 文件在 `/etc/systemd/system` 不在 release 里 → 回滚只回滚二进制，不回滚 unit。正在录制时切换/重启会走 SIGTERM，收尾时长依赖 `TimeoutStopSec`（默认 90 s），文档没写"先停录再切"。
**改法**：写明 `ln -s /opt/tspi-cam/releases/vN current.tmp && mv -T current.tmp current`（rename 原子）；`ExecStart` 用 `current/bin/actiond`；回滚步骤固定为 `actionctl record.stop` → 切链接 → `systemctl restart actiond` → `actionctl hello` 报版本；写明 unit 文件由安装器管理、跨 release 不变（若会变，则 unit 也放 release 并 symlink）；给出 `TimeoutStopSec=` 与"SIGTERM 必须先 finalize 当前录制"。
**测试**：录制中执行回滚脚本，`journalctl -u actiond` 有正常 finalize，`rec-*/index.json` 完整；并发 200 次 `exec` 读 `current/bin/actiond` 不出现 ENOENT。

### S4. 二进制名 `uid` 与仓库产物 `tspi-cam-ui` 不一致，且安装器仍在旧布局 — P2
**位置**：`architecture.md:64`（`bin/{actiond,uid,actionctl,actioncal}`）、`:71`（禁止 `/usr/local/bin`）；对照 `deploy/build.sh:22`、`deploy/install.sh:10-13`、`runtime/ui/Cargo.toml:2`（`tspi-cam-ui`）、`deploy/systemd/actiond.service:8`（`/usr/local/bin/actiond`，且无 `User=`）。
**错在哪**：releases 里的名字没人构建；而现有安装器/unit 正是文档刚宣布禁止的布局。冻结后要么安装器装出"文档里不存在的 uid"，要么两套布局并存。
**改法**：统一名字（建议保留 `tspi-cam-ui` 并改文档，或改 Cargo/脚本为 `uid`），并在同一批改动里重写 `deploy/` 到 releases 布局；文档明确 `actionctl` 符号链接放哪（`/usr/local/bin/actionctl` 仍可，因为它只是入口）。
**测试**：`./deploy/build.sh && ls build/bin` 与文档布局逐名一致；`readlink -f $(command -v actionctl)` 指向 `current/bin/actionctl`。

### S5. 基础层"随安装包替换"与安装器"从不覆盖"矛盾 — P2
**位置**：`architecture.md:52`（更新/回滚：随安装包替换）vs `deploy/install.sh:19-23`（"Never overwrites an existing … toml"）。
**错在哪**：两层配置的全部意义是基础层跟版本走、用户层不动；安装器不覆盖会让基础层永久冻结在首装版本，升级后新默认值/新设备节点永远不生效，且"合并覆盖"的语义错位。
**改法**：基础层由安装器无条件替换（`install -m 0644`），用户层只在首次创建为空表或不存在；写明"用户层键不存在时取基础层，键存在时覆盖；删除用户层键 = 恢复默认"（需要 `device.config.set` 支持删除语义或文档承认没有）。
**测试**：改一次 `config/actiond.toml` 重新安装，`/etc/tspi-cam/actiond.toml` 内容变化而 `/var/lib/tspi-cam/actiond.toml` 不变；`device.config.get` 反映新基础值。

### S6. `uid.service` 的 `After=actiond.service` 不保证 socket 已就绪（`Type=simple`） — P2
**位置**：`architecture.md:81-82`、`:104`（`uid.service` 用 `After=` + `Wants=`）。
**错在哪**：`actiond` 是 `Type=simple`，systemd 在 exec 后即认为 started；`uid` 随即可启动并连接，此时 `bind` 可能还没完成 → `ENOENT`/`ECONNREFUSED`。`ipc.md` 又禁止"静默重试到超时"，所以这会在 UI 首屏变成一次可见错误（或更糟：被当成"服务不可用"永久显示）。
**改法**：`actiond` 改 `Type=notify`（`sd_notify(READY=1)` 放在 bind+chmod/accept 之后），或直接在文档里规定 `uid` 对 `ENOENT/ECONNREFUSED` 的退避重连策略与 UI 状态（"服务启动中" vs "服务不可用"），并把它作为契约的一部分。
**测试**：`systemctl start uid` 与 `actiond` 同时启动 20 次，`uid` 日志中不出现未分类的 ENOENT；`systemd-analyze critical-chain uid.service` 显示 `actiond.service` 的 ready 信号在 `uid` 之前。

### S7. `device.calibrate.enter/leave`（IPC 交接）与 `flock`（停服务交接）是两套机制，且 `Restart=on-failure` 会重新抢锁 — P2
**位置**：`architecture.md:38`（actioncal 走 flock）、`:58`（锁）、`:22-25`（actiond 独占硬件）；对照 `ipc.md:157-158`（`enter` 释放硬件、锁交给 actioncal）。
**错在哪**：两套交接语义不同：IPC 交接时 `actiond` 还活着并持有 socket；flock 交接要求先 `systemctl stop actiond`。文档同时声称两者，标定工具到底用哪条没定。更具体的竞态：`enter` 后若 `actiond` 崩溃，`Restart=on-failure` 会在 2 s 后重启并重新抢锁/开相机，而 `actioncal` 正占着硬件——双开。
**改法**：选 IPC 交接（否则 `enter/leave` 无意义），并写明：`enter` 成功后 `actiond` 进入 `Calibrating` 状态、拒绝 `record.start`、关闭所有设备 fd、把锁的持有者记为 `actioncal`、且 `Restart=` 期间必须先尝试重新取锁，取不到就 `exit(0)` 不重启（或加 `ExecStartPre=/usr/bin/flock -n`）；删除另一条流程。
**测试**：`calibrate.enter` 后 `lsof /dev/video0` 只有 actioncal；`kill -9 actiond` 后 3 s 内不得出现第二个打开 `/dev/video0` 的进程；`calibrate.leave` 后 `record.start` 可用。

### S8. `/userdata/rec` 的属主/模式只剩一句断言，安装器实际建成 `root:root` — P2
**位置**：`architecture.md:102`（"录制目录 `/userdata/rec` 归 `tspi-cam` 服务账号所有"）、`:59`（录制产物）。
**错在哪**：`deploy/install.sh:26` 是 `install -d /userdata/rec`（root 执行 → `root:root 0755`）。以 `tspi-cam` 运行的 `actiond` 建不了 `rec-*`，也不会在第一次录制时给你一句好错误——这属于"装完就废"的类别，且模式（0755/0770）没定，`record.delete` 的权限也没说。
**改法**：安装器 `install -d -o tspi-cam -g tspi-cam -m 0755 /userdata/rec`（并处理已存在的 root 属主：`chown -R` 现有 rec 目录或在文档里说明不迁移），文档写明模式与"父目录 `/userdata` 只需可穿越"。
**测试**：安装后 `stat -c '%U:%G %a' /userdata/rec`；`sudo -u tspi-cam mkdir /userdata/rec/rec-test && rmdir ...`。

### S9. `actionctl` 画在笔记本上，但通道是板上 Unix socket — P2
**位置**：`architecture.md:18`（`你，在笔记本 or CLion ─▶ actionctl`）。
**错在哪**：`/run/tspi-cam/record.sock` 是板上的本地 socket，笔记本上的 `actionctl` 打不开；要么 `ssh board actionctl`，要么 `ssh -L /run/tspi-cam/record.sock:...`（OpenSSH Unix socket 转发）。文档没写，实现者会去加 TCP 监听——那正好破坏 §7 的权限模型。
**改法**：写明两种受支持方式，明确"不开 TCP、不做鉴权，远端一律走 ssh 的 socket 转发"，并给出命令；`actionctl` 的 `--socket` 参数默认值写出来。
**测试**：在笔记本上按文档执行一次 `record.status`；`ss -ltn` 在板上不出现新增监听端口。

---

## 3. Nit

- **N1**（`architecture.md:81`）mermaid 里的 `After=local-fs,dev-video0?` 不是合法 systemd 语法：`local-fs.target`、`dev-video0.device`。建议改成真实 unit 行或干脆只画"systemd → actiond → uid"，细节留给 deploy.md；`/userdata` 若是独立挂载，用 `RequiresMountsFor=/userdata` 比 `After=` 可靠。
- **N2**（`architecture.md:97-98`）`KERNEL=="card[0-9]"` 只匹配单数字、`renderD*` 缺 `SUBSYSTEM=="drm"`；现在能用，但建议写成 `SUBSYSTEM=="drm", KERNEL=="card[0-9]*"` 与 `SUBSYSTEM=="drm", KERNEL=="renderD*"`，并在注释里写"card* 同时覆盖 Xorg 所需节点"。
- **N3**（`architecture.md:73`）"`actionctl` 是 `current/bin/actionctl` 的符号链接"没说链接放在哪；`command -v actionctl` 的答案应由文档给出（建议 `/usr/local/bin/actionctl`，并说明它为什么不算"第二种布局"）。
- **N4**（`architecture.md:38`、`:58`）"只连 `actiond` 的 `device.lock`（flock）"措辞错误：flock 是 open+flock，不是 connect；"谁写：两者"也没说谁在什么条件下创建。
- **N5**（`README.md:24`）参考里的 `docs/design/architecture.md` 看起来像本仓库路径，实际指 microduck 内部；建议写成 `microduck:docs/design/architecture.md`。另外 `README.md:8` 的"已定（2026-11-16）"与 `architecture.md` 里仍带 `?`、依赖 H8/udev 未落盘的状态不符，状态列建议区分"服务划分已定 / 部署机制待 deploy.md"。
- **N6**（`architecture.md:36`）"uid 拥有 HDMI 输出"过强：DRM master 由 Xorg 持有，`uid` 只拥有画面内容；措辞会把"谁打开 `/dev/dri/card0`"这个问题藏起来（与 B3 同源）。

---

## 4. 如果照现在这样冻结，最可能在第几步炸

**第 2 步（第一次 `systemctl start actiond`）就炸，且是两道独立的墙。** 安装器照 `deploy/install.sh` 装出 `/usr/local/bin` + root 运行（与 `:71`/`:87` 冲突），即便手改成 `tspi-cam`，`RuntimeDirectory=` 建的 `/run/tspi-cam` 属主是 `tspi-cam`，而 `:105` 要求 bind 后 `fchown` 到 `root:video` —— 非 root 直接 `EPERM`（WSL 实测），启动失败或 socket 属主与契约不符。同一时刻 udev 只授权了 spi/mpp/v4l/drm：第一次预览缩放打开 `/dev/rga` EACCES、rkisp 配管线打开 `/dev/media0` EACCES；而 `spidev3.0` 按 H5 开机根本不存在，IMU 直接 `E_HW`。

**第 3 步（`uid` 起来）炸第二层。** `k` 没被加入 `video`，即使 socket 属主侥幸对了也连不上；没有 Xorg unit，`DISPLAY` 未定义，libmali 只有 x11 变体（H10）→ EGL 初始化必失败；触摸归属还停在"evdev 还是 Xorg"的岔路口，`uid` 的依赖和组都没法拍板。

**第 4–5 步（第一次录制、第一次标定）炸的是静默类问题**，最难在联调时定位：`/userdata/rec` 是安装器建的 `root:root`，`actiond` 建不了 `rec-*`；配置文件名 `recordd.toml` 让安装器与代码各写各的，升级时基础层又因"从不覆盖"永远停在首装值；标定时 `systemctl stop actiond` 顺手删掉了 `/run/tspi-cam`，`actioncal` 以 `k` 连锁文件都创建不了。到这一步，问题已经从"权限写错"变成"三层状态（文件属主、systemd 目录生命周期、配置分层）互相对不上"，返工的是部署脚本 + 配置模块 + 标定流程三块，而不是改一行 udev。

最省事的冻结前收敛顺序：先定 **B1/B2（socket 属主 + `k` 的组）** 和 **B9（谁拥有部署事实）**，把 udev/systemd 细节整体移进 deploy.md；再补 **B4/B5（rga/media0/spidev-bind）**；最后把 **B7/B8（`/var/lib` 与锁的属主/写者）** 写成一张表——这三组定了，S 类都是照着表填。