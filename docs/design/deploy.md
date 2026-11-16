# 部署、安装与回滚

- 状态：v1（2026-11-16）· 拥有者：本文 = 安装布局、systemd 单元与切换、udev、组、回滚检查表
- 服务划分与"谁拥有什么"见 [architecture.md](architecture.md)；socket 权限表见 [ipc.md](ipc.md) §7。

## 1. 安装布局

```text
/opt/tspi-cam/releases/<version>/
├── bin/{actiond,uid,actionctl,actioncal}
├── systemd/{actiond.service,actiond.socket,uid.service}
└── udev/70-tspi-cam.rules
/opt/tspi-cam/current -> releases/<version>

/etc/systemd/system/actiond.socket     # 安装器写入，指向 current/bin
/etc/systemd/system/actiond.service
/etc/systemd/system/uid.service
/etc/systemd/system/xorg.service        # 镜像/安装器提供（root + -ac）
/etc/systemd/system/spidev-bind.service # 镜像提供（H5：重启后 spidev 节点需要重绑）
/etc/sysusers.d/tspi-cam.conf
/etc/tmpfiles.d/tspi-cam.conf
/etc/udev/rules.d/70-tspi-cam.rules
/etc/tspi-cam/actiond.toml              # 基础配置；安装时存在就不覆盖
```

- unit 文件放在 `/etc/systemd/system` 并指向 `current/bin/*`：**切换版本不需要 `daemon-reload`**。
  若某版本必须改 unit，安装器负责写入并 reload，这一步不算回滚安全点（见 §5）。
- 配置/数据从不进 release：`/etc/tspi-cam`、`/var/lib/tspi-cam`、`/userdata/rec`。

## 2. 账号、组与目录

```ini
# /etc/sysusers.d/tspi-cam.conf
u tspi-cam - "tspi-cam service" /var/lib/tspi-cam
m tspi-cam video
```

```ini
# /etc/tmpfiles.d/tspi-cam.conf
d /run/tspi-cam 0755 root video -
d /var/lib/tspi-cam 0755 tspi-cam video -
d /userdata/rec 0775 tspi-cam video -
```

安装器另外执行（**需要重新登录才生效**）：

```bash
usermod -aG video k      # actionctl/uid 连 socket、读 /dev/dri
# usermod -aG input k    # 仅当 Xorg 改由 k 运行时才执行（当前 xorg.service 以 root 运行）
```

- `actiond.service`：`User=tspi-cam`、`SupplementaryGroups=video`、**不设 `RuntimeDirectory=`**
  （原因见 architecture §3：会破坏 socket 属主）。
- `/var/lib/tspi-cam` 归 `tspi-cam:video **0755**`：写者全是 `tspi-cam`（actiond 写用户层与
  `device.json`，actioncal 以 `tspi-cam` 写 `calibration.json`），组只读即可；
  `device.lock` 也在这里（不是 `/run`）。

## 3. systemd 单元（要点）

```ini
# actiond.socket
[Socket]
ListenStream=/run/tspi-cam/record.sock
ListenStream=/run/tspi-cam/media.sock
SocketMode=0660
SocketUser=root
SocketGroup=video
DirectoryMode=0755

# actiond.service
[Service]
User=tspi-cam
SupplementaryGroups=video
Requires=actiond.socket
After=actiond.socket spidev-bind.service
Wants=spidev-bind.service
ExecStart=/opt/tspi-cam/current/bin/actiond --config /etc/tspi-cam/actiond.toml
Restart=on-failure
RestartSec=2
```

```ini
# uid.service — 客户端，不是录制链路的一部分
[Service]
User=k
Environment=DISPLAY=:0
After=xorg.service actiond.service
Requires=xorg.service
Wants=actiond.service
ExecStart=/opt/tspi-cam/current/bin/uid
Restart=on-failure
```

- `uid` 对 `actiond` 是 `Wants=`：daemon 不在时 UI 起来显示"服务不可用"，不阻塞。
- `xorg.service` 由镜像提供（`root`，`Xorg :0 -ac -nolisten tcp vt7`）；**以 `k` 运行未验证**，
  不要凭想象写 `User=k`（需要 logind seat + tty 权限，见 ui.md 验收项）。

## 4. udev 规则

```udev
KERNEL=="spidev3.0", GROUP="video", MODE="0660"
KERNEL=="mpp_service", GROUP="video", MODE="0660"
SUBSYSTEM=="video4linux", GROUP="video", MODE="0660"
KERNEL=="media[0-9]*", SUBSYSTEM=="media", GROUP="video", MODE="0660"
KERNEL=="card[0-9]", SUBSYSTEM=="drm", GROUP="video", MODE="0660"
KERNEL=="renderD*", SUBSYSTEM=="drm", GROUP="video", MODE="0660"
KERNEL=="rga", GROUP="video", MODE="0660"
```

- 安装后：`udevadm control --reload && udevadm trigger`。
- `spidev3.0` 只在 `spidev-bind.service` 绑定后出现，规则对"后出现"的设备同样生效；
  `actiond.service` 的 `After=/Wants=spidev-bind.service` 覆盖开机顺序。
- **未验证项**：`/dev/dma_heap/*`、`/dev/ion` 是否需要同组访问（v2 零拷贝才用）。
  验收命令：`sudo -u tspi-cam test -r /dev/dma_heap/system && echo ok`。没有这条，
  "全部设备由 udev 授权"就是假命题——写进 §6 检查表。

## 5. 切换与回滚

```bash
# 安装新版本（同一文件系统上的 rename，没有 unlink 窗口）
tar -C /opt/tspi-cam/releases -xf release-<v>.tar
ln -s releases/<v> /opt/tspi-cam/current.tmp
mv -T /opt/tspi-cam/current.tmp /opt/tspi-cam/current
systemctl restart actiond.service uid.service

# 回滚
ln -s releases/<prev> /opt/tspi-cam/current.tmp
mv -T /opt/tspi-cam/current.tmp /opt/tspi-cam/current
systemctl restart actiond.service uid.service
```

- 保留 `current` + 上一个版本；健康检查通过后再删更旧的。
- 切换前**必须**先 `record.stop`（或 `systemctl stop actiond` 让其 Finalize）；
  正在录制时直接 restart 会让当前记录 `complete=false`。
- `TimeoutStopSec` 要足够覆盖 Finalize（1080p/5MP 5 s 级），建议 30 s；`KillMode=mixed`
  让 `actiond` 自己收尾。

## 6. 验收检查表（安装/升级后各跑一次）

| # | 检查 | 期望 |
|---|---|---|
| 1 | `systemctl start actiond.socket actiond.service` | 无 `EPERM`，`journalctl -u actiond` 无 error |
| 2 | `stat -c '%a %U:%G' /run/tspi-cam/*.sock` | `660 root:video` |
| 3 | `sudo -u k /opt/tspi-cam/current/bin/actionctl hello` | 返回 `api_version` |
| 4 | `sudo -u tspi-cam test -r /dev/video0 -a -w /dev/mpp_service -a -r /dev/spidev3.0 -a -r /dev/rga` | 全可读/写 |
| 5 | `stat -c "%U:%G %a" /var/lib/tspi-cam && sudo -u tspi-cam touch /var/lib/tspi-cam/.probe` | `tspi-cam:video 755`，可写 |
| 5b | `sudo -u tspi-cam test -r /dev/dma_heap/system` | `UNVERIFIED`（v2 才需要；失败记录，不阻塞 v1） |
| 6 | `actionctl record.start` + 5 s + `record.stop` | `index.json.complete == true`，`frames > 0` |
| 7 | 回滚到上一版并重复 1–3 | 全部通过，旧录制仍可 `record.list` |
| 8 | `systemctl is-active spidev-bind.service` | `active (exited)`；失败时 `actiond` 报 `E_HW` 而不是崩溃 |
