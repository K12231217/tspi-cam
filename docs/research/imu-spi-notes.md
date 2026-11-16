# IMU660RB / LSM6DSR on SPI3：板上事实与坑

- 日期：2026-11-16 · 板：`k@192.168.31.129`（Ubuntu 20.04.6，kernel 4.19.232 aarch64）
- 来源：`https://gitee.com/lik1122/taishanpi-tools`（`spi_imu660rb_test.py`、`imu_block_read.py`、
  `imu_raw_check.py`、`spidev-bind.service`）+ 本仓 §16 H3/H5/H6
- 影响：`action-core::imu` 的实现与 `deploy.md` 的 udev/unit 清单

## 1. 症状

- 记录器的 WHO_AM_I 与采样计数正常，但分析出来的值不对：**gyro X 冻结在约 +110 °/s**，
  **加速度模长约 0.96**（应为 9.81）。
- 同一批寄存器，**块读**与**单字节读**结果不一致；块读的 gyro X 卡在 +12544、accel Z 卡在 +1600，
  而单字节读给出 temp ≈21.5 °C、gyro ≈0、|accel| ≈1 g（正确）。

## 2. 结论（板上实测，已复现）

> **本板的多字节自增读（burst read）返回冻结/错位的数据；单字节读正确。**

因此 IMU 采样**必须逐字节读**：每个寄存器一次 2 字节 SPI 传输（`reg|0x80, 0x00`），
一帧 12 个寄存器 = 12 次 ioctl。这不是优化项，是正确性要求；块读哪怕快 10 倍也不能用。
`action-contract::imu::MULTI_BYTE_READ_BROKEN = true` 与 `interfaces.md` §6 写的是同一个事实。

## 3. 节点与权限（部署层）

| 事实 | 后果 |
|---|---|
| 重启后 `/dev/spidev3.0` **消失**（driver_override 绑定不持久，H5） | 需要 `spidev-bind.service`：先 `modprobe spidev`，等 `spi3.0` 出现，然后写 `driver_override` 并 `bind` |
| 节点默认 `0600 root:root` | 以 `tspi-cam` 运行的 `actiond` 打不开；`deploy.md` §4 的 udev 规则（`KERNEL=="spidev3.0", GROUP="video", MODE="0660"`）是必需项，不是可选 |
| 诊断脚本都要求 `sudo python3 …` | 与上一条同因；不要用 sudo 运行 `actiond`（那会破坏 socket 权限模型） |

## 4. 寄存器配置（datasheet 值，非本板实测）

| 寄存器 | 值 | 含义 |
|---|---|---|
| `WHO_AM_I` (0x0F) | `0x6B` | LSM6DSR 家族（H3 板上已验证） |
| `CTRL1_XL` (0x10) | `0x50` | ODR 208 Hz，±2 g（灵敏度 0.061 mg/LSB） |
| `CTRL2_G` (0x11) | `0x50` | ODR 208 Hz，±250 dps（8.75 mdps/LSB） |
| `CTRL3_C` (0x12) | `0x44` | BDU + IF_INC |
| `STATUS_REG` (0x1E) | bit1 GDA / bit0 XLDA | 新数据标志（M1 轮询不用，M2 DRDY 用） |
| 输出 | `0x22..0x27` gyro、`0x28..0x2D` accel | 各轴 little-endian i16 |

`taishanpi-tools` 的诊断脚本用 `0x60`（416 Hz）只是为了快速看原始值；生产配置按契约用 **208 Hz**
（LSM6DSR 没有 200 Hz 档，200 Hz 只是轮询目标，不得写进硬件 ODR 或 `index.json.odr_hz`）。

## 5. 本仓 Rust 驱动在板上的验证（2026-11-16）

交叉编译（WSL，静态 musl，避开 24.04 的 glibc 2.39 与板端 2.31 的 mismatch）：

```bash
rustup target add aarch64-unknown-linux-musl
CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER=rust-lld \
  cargo build --release --target aarch64-unknown-linux-musl --example imu_probe
scp target/aarch64-unknown-linux-musl/release/examples/imu_probe k@<board>:/tmp/
ssh k@<board> 'sudo /tmp/imu_probe /dev/spidev3.0 10'
```

实测输出（静置，连续 10 帧）：

```text
open /dev/spidev3.0: WHO_AM_I=0x6B odr=208Hz
#0 ... gyro(rad/s)=[0.0027,-0.0078,-0.0020] accel(m/s2)=[-0.891,1.866,9.614] raw=[18,-51,-13,-1489,3120,16072]
#9 ... gyro(rad/s)=[0.0035,-0.0093,-0.0018] accel(m/s2)=[-0.917,1.900,9.613] raw=[23,-61,-12,-1533,3176,16070]
```

结论：`|accel| ≈ 9.81 m/s²`、gyro ≈0、相邻帧间隔 ≈5.9 ms（探针每帧 sleep 5 ms），
**没有** gitee 仓库里"gyro X 冻结 +110 °/s、|accel| 0.96"的症状 —— 单字节读路径正确。
`/dev/spidev3.0` 当前仍是 `0600 root:root`，探针用 `sudo` 跑；正式部署靠 udev 规则。

## 6. 复核方法

```bash
# 板上（节点存在的前提是先装 spidev-bind.service）
sudo /tmp/imu_probe /dev/spidev3.0 5     # 本仓 examples/imu_probe.rs
# 输出应 WHO_AM_I=0x6B，静置时 gyro≈0、|accel|≈9.81，转动时 raw 明显变化
```

若 `open failed: ... Permission denied` 而节点存在：udev 规则没装（见 §3）；
若 `spidev3.0` 不存在：`spidev-bind.service` 没装或没等够 SPI 控制器枚举。
