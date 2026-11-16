# tspi-cam crate 边界与接口

- 状态：v2（2026-11-16，按 review 修订）· 拥有者：本文 = "crate 怎么分、trait 长什么样、谁依赖谁"
- 线上协议见 [ipc.md](ipc.md)，服务划分见 [architecture.md](architecture.md)。
- 修订要点：`FrameLease` 改为**拥有式**（不再借用 `FrameSource`）；补 Buffer Pool 与编码器输入归还
  契约；修正 NV12 常量；补分辨率缩放契约；crate 先不拆细；去掉全局 `cargo` target。

## 1. Workspace 划分（先一个 core，七个模块）

review 结论：**在第一份可运行实现之前不拆六个 crate**。边界靠目录 + 依赖门禁，不靠 crate 数量。

| crate | 类型 | 拥有 | 允许依赖 |
|---|---|---|---|
| `action-ipc` | lib | 线上契约：`Call`/`Event`、方法名、错误码、socket 常量、帧头、**serde 夹具测试** | serde, serde_json, semver |
| `action-contract` | lib | 器件事实：传感器模式、NV12/stride 公式、编码器上限、IMU 寄存器；**零依赖** | 无 |
| `action-config` | lib | 配置 schema、默认值、两层合并（`/etc` + `/var/lib`）、校验 | ipc, serde, toml |
| `action-core` | lib | 采集/编码/IMU/写盘/时钟/存储/缩放，按模块划分：`capture/ encode/ imu/ record/ clock/ storage/ scale/` | ipc, contract, config |
| `actiond` | bin | 服务：状态机、IPC server、设备锁、事件推送 | core, ipc, config |
| `uid` | bin | UI：Xorg/egui、触摸、预览订阅 | ipc |
| `actionctl` | bin | CLI：所有 `Call` 的薄客户端 | ipc, config |
| `actioncal` | bin | 离线标定：抢锁、采数、写 `calibration.json` | core, contract, config |
| `xtask` | bin | 门禁与打包 | 无（只读仓库） |

`action-core` 内部模块边界（禁止互相反向依赖，`xtask arch` 检查 `use`）：

```text
clock ◄── capture ◄── scale ◄── encode
  ▲         ▲                      ▲
  └── imu   └────── record ◄───────┘
storage ◄── record
session.rs（装配层）：只有它能同时 use capture/encode/imu/record/scale
```

**Host 可编译是硬要求**：所有 OS 相关代码（v4l2/MPP/spidev/DRM）放在
`#[cfg(target_os = "linux")]` 模块里；协议、状态机、`Index` 序列化、Buffer Pool 记账、
配置合并必须在 **host（WSL/x86_64）** 上 `cargo test` 可跑。板上只跑集成验证。

因此仓库**不设全局 `[build] target`**（那会把 `xtask`、host 单测、任何笔记本侧工具一起重定向）。
目标按调用选择：`cargo board` / `cargo board-check` / `cargo board-test`（`.cargo/config.toml` 里的
alias）或 `cargo xtask` 内部显式传 `--target`。host 测试与板端验证的分工写进 `gates.md`。

## 2. 契约层（`action-contract`，零依赖）

```rust
/// IMU660RB = LSM6DSR 家族，WHO_AM_I(0x0F) 期望 0x6B（器件手册 + 板上实测 H3）
pub const IMU_WHO_AM_I_REG: u8 = 0x0F;
pub const IMU_WHO_AM_I: u8 = 0x6B;
/// 硬件 ODR 档位：LSM6DSR 无 200 Hz，最近档 208 Hz（H3/H6）
pub const IMU_ODR_HZ: u32 = 208;

/// OV5695 当前 BSP 只暴露一个模式（media-ctl 实测 H14）
pub const SENSOR_W: u32 = 2592;
pub const SENSOR_H: u32 = 1944;
pub const SENSOR_FPS: u32 = 30;

/// NV12 平面尺寸（含步幅）。stride 来自 V4L2 `plane_fmt[0].bytesperline`。
pub const fn nv12_luma_len(stride: u32, h: u32) -> usize { stride as usize * h as usize }
pub const fn nv12_total(stride: u32, h: u32) -> usize { nv12_luma_len(stride, h) * 3 / 2 }

// 来源：`v4l2-ctl --get-fmt-video` 在 2592×1944 报 bytesperline=2592、sizeimage=7_558_272（本板实测）
const _: () = assert!(nv12_total(2592, 1944) == 7_558_272); // 2592*1944*1.5
const _: () = assert!(nv12_total(1920, 1080) == 3_110_400);
const _: () = assert!(nv12_luma_len(2592, 1944) == 5_038_848);

/// MPP 编码器实测上限（H12/H15/H16）；策略分辨率另由配置决定
pub const ENC_FPS_CEIL_5MP: u32 = 30;      // 2592×1944 实测 30.045 fps
pub const ENC_FPS_CEIL_1080P: u32 = 36;
pub const ENC_FPS_CEIL_720P: u32 = 81;
```

> 上一版把 `2592×1944×1.5` 写成 `7_552_272`，少了 6000 字节；同时把 1080p 的
> `60×3_110_400 = 186_624_000` 误当成 5MP 的数据。两处均已按本板 `v4l2-ctl` 实测值修正，
> 并加了 1080p 的对照断言，防止再漂。

## 3. 采集：拥有式 `FrameLease` + Buffer Pool

review 的核心问题：`fn next_frame(&mut self) -> FrameLease<'_>` 表达的是**借用**
（借用期间不能再次 `next_frame`，也无法跨线程），而我们需要的是**跨线程传递的缓冲所有权**。
因此：

```rust
/// 池化的 V4L2 buffer。Pool 拥有 fd（内部 dup）与全部 mmap，且必须 Send + Sync。
pub struct BufferPool { /* Arc<PoolInner> */ }

impl BufferPool {
    /// fd 所有权归池（内部 dup）：`stop()` 不关 fd，池析构才关。
    pub fn new(fd: std::os::fd::BorrowedFd<'_>, count: u32) -> Result<Self, CaptureError>;
    /// 一步完成 DQBUF + 构造租约。pts/id 只在这里有来源，所以不提供两段式 API。
    pub fn dqbuf(&self, timeout: Duration) -> Result<Option<FrameLease>, CaptureError>;
    pub fn available(&self) -> usize;          // 原子读；精确值用 stats()
    pub fn stats(&self) -> PoolStats;          // dequeued, returned, return_errors, gap_frames
    /// 只做 VIDIOC_STREAMOFF：**不 munmap、不 REQBUFS(0)、不 close**。
    pub fn stop(&self) -> Result<(), CaptureError>;
}

pub struct FrameLease {
    pub id: u64,            // = v4l2_buffer.sequence（驱动帧序号，丢帧缺口由它算）
    pub pts: MonoTime,      // = timebase.from_driver(v4l2_buffer.timestamp, src)
    pub geom: Geometry,
    pub format: PixelFormat,
    pub stride: u32,        // 见 §3.2
    planes: FramePlanes,
    buf: BufferIndex,
    pool: Arc<PoolInner>,
}
impl FrameLease {
    /// 视图生命周期绑定 `&self`；不存在 `&'static` 版本（否则 lease drop 后仍可读，见 review E3）。
    pub fn planes(&self) -> FramePlanes<'_>;
}
impl Drop for FrameLease {
    /// 槽位 CAS: Leased -> Free，然后 QBUF 一次；失败计入 stats.return_errors，不 panic。
    fn drop(&mut self) { /* … */ }
}
unsafe impl Send for FrameLease {}   // 前提：PoolInner: Send + Sync，槽位状态 AtomicU8
// 同时断言 FrameLease: Send 且 !Sync（static_assertions），因为多线程共享同一 lease 没有意义且危险。

pub trait FrameSource: Send {
    fn start(&mut self, geom: Geometry, fps: u32) -> Result<SourceInfo, CaptureError>;
    fn next_frame(&mut self, timeout: Duration) -> Result<Option<FrameLease>, CaptureError>;
    /// 只 STREAMOFF；已发出的 lease 仍然可读，munmap 只发生在池析构。
    fn stop(&mut self);
    fn info(&self) -> SourceInfo;
}
```

### 3.0 安全前提与不变式（不写清楚就不能加 `unsafe`）

- **`PoolInner: Send + Sync` 是显式承诺**：`base: *mut u8` 等裸指针字段需要 `unsafe impl`，槽位状态用
  `AtomicU8`（`Free → Dequeued → Leased → Free`，CAS 迁移）。用 `Cell<usize>` 记账会丢 5–8% 的更新
  （review 实验 E2：20 万次并发，期望 1,000,000 次计数，实测 936k–949k），`available()` 与
  `qbuf_calls` 就永远对不上。
- **同一 buffer 不可能被租两次**：`FrameLease` 只在 `dqbuf` 内部构造（`lease()` 是私有、且不导出），
  所以"两次 Drop 两次 QBUF"在类型层面不可达。
- **`stop()` 不改内存生命周期**：只 STREAMOFF。`munmap` / `REQBUFS(0)` / `close` 只发生在
  `PoolInner::drop`。`stop()` 之后 `next_frame` 返回 `Err(CaptureError::Stopped)`（不是 `Ok(None)`）；
  此时已发出的 lease 仍可读，`Drop` 里的 QBUF 允许返回 `ENODEV/EBADF`，计入 `return_errors`。
- **容量不变式**：`count >= queue_depth + encoder_in_flight_max + 2`。本板 `count=8`，因此
  **`queue_depth ≤ 4`（≈133 ms @30 fps）**，不是 8。池里 8 个 buffer 被应用全持有时驱动没有可写
  buffer，采集会被消费者节流——表现为 fps 抖动而不是 `dropped++`，这正是必须留余量的原因。
- **`dropped` 有两个来源，分开计数**：`gap_frames`（`sequence` 缺口，驱动侧丢）与
  `queue_dropped`（应用侧溢出）。`record.event` / `index.json` 都分别记录，不允许合并成一个数。

### 3.1 生命周期与所有权

```text
capture: dqbuf → lease (InFlight) ──queue──▶ encode: submit(lease) ──▶ 等该帧的 packet
                                                    │                        │
                                                    └────── 归还 ◄───────────┘
```

- **`submit()` 返回不代表硬件读完**。MPP 是异步的：`encode_put_frame` 之后硬件在任务里读输入。
  契约规定：**编码器内部保存输入 lease 的 FIFO；每次 `poll()` 收到一个 packet，就释放 FIFO 头部
  的 lease**，从而实现"packet 出 → 输入 buffer 可归还"。
- 前提：`bframes = 0` 且编码器对每个输入帧恰好产生一个 packet。**这条要在板上验证**（600 帧：
  `submits == packets`，顺序一致）。若不成立，`poll()` 返回
  `EncodeError::LifecycleMismatch`（`E_ENC_LIFECYCLE`），**不静默丢帧**。
- `flush()`：发 EOS 后把 FIFO 里剩余 lease 全部归还，再返回；期间 `poll()` 允许返回 `None`。

### 3.2 步幅（stride）的精确定义

```text
stride = V4L2 plane_fmt[0].bytesperline        // 单位：字节；本板 2592×1944 实测 = 2592
luma plane   = [0, stride*h)                   // 有效行宽 w，行尾 [w, stride) 是填充
chroma plane = [stride*h, stride*h + stride*(h/2))
total        = stride*h*3/2                    // 实测 sizeimage = 7_558_272，与上式一致
```

- `w` 与 `stride` 是**两个不同的量**：`planes()` 返回整段 `stride*h`，消费方必须用
  `stride` 跳行，不能用 `w`。
- 编码器收到 `stride != w` 时必须显式设置水平步幅（`mpp_frame_set_hor_stride`）；
  RGA 路径同样按 stride 传。**任何"顺手重新打包成 w 紧凑排布"的实现都算违约**（多一次全帧拷贝）。
- 若驱动给出 `stride < w`，视为 `CaptureError::BadStride`，不允许继续。

## 4. 编码接口与输入归还

```rust
pub struct EncoderConfig {
    pub codec: Codec,          // H264 | H265
    pub geom: Geometry,        // 编码输出分辨率（白名单见 §5）
    pub bitrate_kbps: u32,
    pub gop: u32,
    pub bframes: u8,           // 契约要求 0
}

pub struct EncodedPacket { pub pts: MonoTime, pub keyframe: bool, pub data: Vec<u8> }

pub trait VideoEncoder: Send {
    fn open(cfg: &EncoderConfig) -> Result<Self, EncodeError> where Self: Sized;
    /// 取得输入帧所有权；返回即意味着 lease 归编码器所有，调用方不得再用。
    fn submit(&mut self, frame: FrameLease) -> Result<(), EncodeError>;
    /// 阻塞至多 timeout；返回 packet 时，FIFO 头部输入已归还（Drop lease）。
    fn poll(&mut self, timeout: Duration) -> Result<Option<EncodedPacket>, EncodeError>;
    fn submit_eos(&mut self) -> Result<(), EncodeError>;
    fn flush(&mut self) -> Result<(), EncodeError>;
    fn stats(&self) -> EncoderStats;   // submits, packets, in_flight, lifecycle_errors
}
```

- 编码器**只能**通过 `FrameLease` 取得输入；不接受 `&[u8]`，避免绕过池的裸指针。
- `submit` 后调用方不得访问该帧；想读就用 `stats()` 或自己复制后再提交（不推荐）。
- 生命周期配对失败计入 `stats.lifecycle_errors` 并触发 `E_ENC_LIFECYCLE`，录制进 `Fault`（§7 旧文档状态机）。

## 5. 分辨率契约（sensor / encode / preview 三个几何）

review 指出的缺口：5MP 传感器输入与 1080p 输出之间没有写明谁缩放、什么时候丢帧。

| 名称 | 取值 | 谁决定 | 说明 |
|---|---|---|---|
| `sensor.geom` | 2592×1944 @30 | 驱动（唯一模式，H14） | 采集固定在此，**不尝试改传感器分辨率换帧率** |
| `encode.geom` | 白名单 `{2592×1944, 2560×1440, 1920×1080, 1280×720}` | 配置 | 默认 1920×1080；必须满足 `fps ≤ ENC_FPS_CEIL_*` |
| `preview.geom` | 480×800（面板原生，可 letterbox） | UI | 由预览路径产出，**不参与录制** |

- `encode.geom == sensor.geom`：零缩放，直通（5MP 30 fps，实测达标）。
- 否则插入 **RGA** 缩放（`scale` 模块，硬件 ioctl 同步完成）：capture lease → RGA blit →
  RGA 输出 buffer（第二个池）→ 编码器。RGA 完成即归还 capture lease（同步语义，安全）。
- **RGA 吞吐尚未实测**（`UNVERIFIED`）：验收条件 = 1080p 缩放 + 编码合计 ≥30 fps、且
  `v4l2` 不丢帧；达不到就退化为"录制按 sensor.geom、预览单独缩放"。
- 预览：从 encoder 前的帧按固定比例抽帧（默认 10 fps）缩小到 480×800；**CPU 缩放实现不允许**
  （会偷走编码预算），用 RGA 或让 MPP 直接出小图。
- 配置校验（`action-config`）：`encode.geom` 必须在白名单、且 `fps ≤ ceiling`；否则 `E_CFG`。

## 6. IMU 接口

```rust
pub struct ImuSample {
    pub t: MonoTime,        // t_spi_read；M2 可换 t_drdy
    pub gyro: [f32; 3],     // rad/s
    pub accel: [f32; 3],    // m/s²
    pub raw: [i16; 6],      // 原始 LSB（保留以便离线重算）
}

pub trait ImuSource: Send {
    fn open(&mut self, spi: &str, odr_hz: u32) -> Result<ImuInfo, ImuError>;
    fn read(&mut self) -> Result<ImuSample, ImuError>;
    fn last_timing(&self) -> (MonoTime, MonoTime);
}
```

硬约束：**单字节读**（H6：多字节自增读返回错数据）；ODR 默认 208（配置项）；换算系数在
`action-contract`。

## 7. 写盘接口

```rust
pub struct RecordConfig {
    pub dir: PathBuf,          // /userdata/rec
    pub encode: EncoderConfig,
    pub headroom_mb: u64,
}

pub struct Recorder { /* 两个独立线程：video / imu */ }

impl Recorder {
    pub fn begin(cfg: RecordConfig) -> Result<(Self, RecId), RecordError>;
    pub fn video(&mut self, pkt: &EncodedPacket) -> Result<(), RecordError>;
    pub fn imu(&mut self, s: &ImuSample) -> Result<(), RecordError>;
    pub fn finish(self) -> Result<Index, RecordError>;  // Drain→fsync→index.json.tmp→rename→fsync(dir)
    pub fn abort(self) -> Index;                        // 尽力落盘，complete=false
}
```

## 8. 门禁（`cargo xtask`，单一构建入口）

| 子命令 | 检查 | 负向验证（必须看到红） |
|---|---|---|
| `xtask arch` | `cargo tree` 与模块 `use` 方向；`action-contract` 零依赖；`action-ipc` 只依赖 serde | 给 `action-ipc` 加 workspace 依赖 → 红 |
| `xtask ipc` | [ipc.md](ipc.md) §8 六条；文档 JSON 块 = 测试夹具 | 改文档示例不改测试 → 红；删 `deny_unknown_fields` → 红 |
| `xtask test` | host（WSL/x86_64）`cargo test`：状态机、协议往返、Buffer Pool 记账、Index 序列化 | 让 `return_buffer` 少调一次 → 红 |
| `xtask docs` | 一机制一页；链接有效 | 两页重复写同一机制 → 红 |
| `xtask smoke` | 板上：`record.status` → `record.start` 5 s → `record.stop`；核对帧数、`index.json`、退出码 | 把 `record.sock` 指到不存在路径 → `E_IPC` 而非超时 |
| `xtask size` | 产物大小上限 | — |

## 9. Buffer Pool 的验收测试（host 可跑，mock 驱动）

所有断言都针对**精确计数**，不是"没崩就算过"：

1. **容量与余量**：`count=8, queue_depth=4`；连续 `dqbuf` 8 次全部成功，第 9 次返回 `Ok(None)`（超时）；
   归还 1 个后恢复。另有一条不变式测试：`assert!(count >= queue_depth + max_in_flight + 2)`。
2. **Drop 即归还且只还一次**：N=10_000 次 lease/drop 后 `stats.returned == 10_000`、
   `stats.return_errors == 0`、`qbuf_calls == 10_000`（mock 计数）。用 `Cell` 记账的实现会在此红
   （review E2：并发下丢 5–8%）。
3. **跨线程**：`assert_send::<FrameLease>()` + `assert_not_impl_any!(FrameLease: Sync)`
   （**只断言 Send 没用**：`unsafe impl` 会让它恒过，review E1）；生产者/消费者 10k 次往返，
   `qbuf_calls == 10_000 && available() == count`。
4. **`stop()` 不动内存**：持有 1 个 lease 时 `stop()` → `munmap_calls == 0` 且 `planes()` 仍可读；
   drop lease 后 `returned == 1`；最后 drop 池 → `munmap_calls == 1`。`stop()` 后 `next_frame`
   必须返回 `Err(CaptureError::Stopped)`，不是 `Ok(None)`。
5. **fd 归属**：`BufferPool::new` 之后关掉调用方 fd，池仍能 QBUF（内部 dup 生效）；池析构后
   `close` 恰好一次。
6. **驱动丢帧计数**：mock 的 `sequence` 跳号（…, 41, 45, …）→ `stats.gap_frames == 3`，且
   `record.event` 里 `dropped` 的两个来源分别上报（`gap_frames` / `queue_dropped`）。
7. **编码器配对**：mock 编码器提交 N 帧、吐 N 个 packet，`in_flight == 0`；故意少吐一个 packet →
   `lifecycle_errors == 1` 且返回 `E_ENC_LIFECYCLE`（不允许静默丢）。
