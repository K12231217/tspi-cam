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
/// 池化的 V4L2 buffer。映射由 Pool 持有（Arc），lease 只带索引与视图。
pub struct BufferPool { /* Arc<PoolInner> */ }

impl BufferPool {
    pub fn new(fd: RawFd, count: u32) -> Result<Self, CaptureError>; // REQBUFS + QUERYBUF + mmap
    pub fn dqbuf(&self, timeout: Duration) -> Result<Option<BufferIndex>, CaptureError>;
    pub fn lease(&self, idx: BufferIndex) -> FrameLease;   // 状态 Free -> InFlight
    pub fn available(&self) -> usize;
}

pub struct FrameLease {
    pub id: u64,
    pub pts: MonoTime,
    pub geom: Geometry,
    pub format: PixelFormat,
    pub stride: u32,          // 见 §3.2；来自驱动 bytesperline
    planes: FramePlanes,      // (offset, len) 指向池映射，不含裸指针
    buf: BufferIndex,
    pool: Arc<PoolInner>,
}

impl Drop for FrameLease {
    fn drop(&mut self) { self.pool.return_buffer(self.buf); } // QBUF 回驱动
}
unsafe impl Send for FrameLease {} // 保证：池存活期间映射有效；QBUF 只在 Drop 里发生一次

pub trait FrameSource: Send {
    fn start(&mut self, geom: Geometry, fps: u32) -> Result<SourceInfo, CaptureError>;
    fn next_frame(&mut self, timeout: Duration) -> Result<Option<FrameLease>, CaptureError>;
    fn stop(&mut self);
    fn info(&self) -> SourceInfo;
}
```

- **`next_frame` 不再有生命周期参数**，返回的 `FrameLease` 可移动到编码线程。
- 采集线程每次只持有"当前正在 DQBUF 的那个"引用；队列里放的是已 lease 的帧，**不阻塞下一次
  `next_frame`**（池容量 8，队列深度 8，满了按 §5 丢帧并计数）。
- 池的映射归 `BufferPool` 所有；`FrameLease` 只保存索引与 `(offset,len)`。**Drop 顺序由
  `Arc` 保证**：租约先于池析构（`actiond` 的线程 join 顺序写进 session）。
- `Send` 是显式 unsafe 承诺，配套测试：`fn assert_send<T: Send>() {}` 静态断言 + 多线程搬运测试。

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

## 9. Buffer Pool 的验收测试（host 可跑）

1. **容量**：`count=8`，租约 8 个后 `dqbuf` 返回 `None`（超时），归还 1 个后恢复；
2. **Drop 即归还**：`lease.drop()` 后 `available()` +1，且 `QBUF` 只发生一次（mock 记录调用次数）；
3. **跨线程**：`FrameLease: Send` 静态断言 + 生产者/消费者线程搬运 10k 次，无泄漏、无双重归还；
4. **池先析构**：线程 join 后池 Drop，无 use-after-unmap（用 mock 计数断言）；
5. **解码/编码配对**：mock 编码器提交 N 帧、吐 N 个 packet，断言 `in_flight == 0`；
   故意少吐一个 packet，断言 `lifecycle_errors == 1` 且返回 `E_ENC_LIFECYCLE`。
