# `docs/design/interfaces.md` §3–§5 / §9 对抗性 review

审查对象：`adca9f8`（相对 `6a72079`）。全部结论只依赖仓库内文本 + 我在 WSL 上用 `rustc 1.100.0` 跑的三个 std-only 实验（不需要 crates.io），没有板子。

**实验证据（先摆出来，后面直接引用）**

- **E1**：`PoolInner { base: *mut u8, fd: RawFd }` + `FrameLease { pool: Arc<PoolInner> }`，只加文档那行 `unsafe impl Send for FrameLease {}`，把 `BufferPool` 移进采集线程 → 编译失败：`error[E0277]: *mut u8 cannot be shared between threads safely ... required for Arc<PoolInner> to implement Send`（`Cell<usize>` 同样两条）。**即文档只写了一个 `unsafe impl`，而实际还需要 `PoolInner: Send + Sync`，文档没有任何一处交代。**
- **E2**：按 E1 补齐 `unsafe impl Send/Sync for BufferPool`，state 用 `Cell<usize>`（即 "Free → InFlight" 的朴素写法），采集线程 `lease()` + 编码线程 `Drop` 并发 20 万次（期望 1,000,000 次计数）：实测 `949836 / 945613 / 936491`，`-O` 下 `943909 / 917836 / 949246`。**丢更新 5–8%，这就是 QBUF 计数与 `available()` 会漂的机制。**
- **E3**：给 `FrameLease` 一个 `fn planes(&self) -> &'static [u8]`（把池映射借成 `'static`，是"不借 `&self` 又要给编码器读"的最自然写法），safe 代码在 lease 与池都析构后仍可读该切片，实测输出 `114` 而不是原值 `171`。**`&'static` 视图 + `Arc` 字段不是安全组合，`planes` 必须只能借 `&self` 生命周期。**

---

## Blocking（冻结前必须改）

### B1. `dqbuf → lease` 两段式 API 同时丢时间戳/序号、不可 RAII、且 `lease()` 不可失败
`interfaces.md:85-90`

三件事叠在同一对签名上：

1. `dqbuf` 只返回 `BufferIndex`，**把 `v4l2_buffer.timestamp` 与 `sequence` 丢掉了**。但 `FrameLease.pts`（line 94）只能在这里取得——`lease(idx)` 手上没有 `v4l2_buffer`。后果：`pts` 无来源，`EncodedPacket.pts`（line 165）无来源，`frames.csv` 的 `frame_id,t_mono_ns`（`docs/architecture.md` §8.1）与"用 sequence 判驱动丢帧"（`docs/architecture.md` §16 H9/H14 的 27–28 fps 与 index.json 的 `frames_dropped`）全部落空。
2. `dqbuf` 返回的 `BufferIndex` 不是 RAII：调用方 `dqbuf` 后忘了 `lease`（或 `?` 提前返回），该 buffer 已出队、永远不会 QBUF，**池容量永久少一个，`available()` 无任何报错地漂移**。
3. `lease(&self, idx) -> FrameLease` 不可失败。公开的 `BufferIndex` + 不可失败的 `lease` 意味着"同一个 idx lease 两次"是一个类型系统看不见的 `unsafe impl Send` 契约违反——两次 Drop 两次 QBUF（`Drop` 注释 line 106 自称"QBUF 只发生一次"，这个保证没有任何机制支撑）。

**改法**：`pub fn dqbuf(&self, timeout: Duration) -> Result<Option<FrameLease>, CaptureError>`，`lease` 降为私有；`FrameLease` 在 `dqbuf` 里一次构造好 `pts/id/geom/stride`，`id` 明确定义为 `v4l2_buffer.sequence`。若坚持两段式，则 `lease` 必须 `-> Result<FrameLease, CaptureError>` 并检查"该 idx 当前在 Dequeued 状态"。

**证明测试**（mock 驱动）：mock 对 idx=3 返回 `sequence=42, timestamp=12345`，断言 `dqbuf` 得到的 `FrameLease.id == 42`、`pts == timebase.from_driver(12345, TsSource::…)`；另一条：`dqbuf` 返回后直接 `drop`（不 lease）8 次，断言池不进入"永不再 dqbuf 成功"的状态（当前 API 下这条测试写不出来，因为失败行为就是它要证明的东西）。

### B2. `unsafe impl Send` 的安全前提写错了，真实前提（`PoolInner: Send + Sync` + 原子槽位状态）文档一条都没有
`interfaces.md:103-106`

注释说"保证：池存活期间映射有效；QBUF 只在 Drop 里发生一次"。这不是 `unsafe impl Send` 需要证明的东西。它需要证明的是：lease 搬到编码线程后，其 `Drop` 里的 `return_buffer(&self)` 与采集线程的 `dqbuf(&self)`/`lease(&self)` 并发访问同一个 `PoolInner` 不会产生数据竞争。E1 证明这个 impl 会把编译器唯一的那道检查（`Arc<PoolInner>: Send` → `PoolInner: Send + Sync`）抹掉；E2 证明用最自然的 `Cell` 记账真会丢 5–8% 的更新，`available()` 于是永远不等于期望值，§9.1/§9.2/§9.5 全部不可判。

**改法**：明确写 `PoolInner: Send + Sync`（`base: *mut u8` 需要 `unsafe impl`），每槽位状态用 `AtomicU8`（或一把 `Mutex` 串行化记账 + QBUF），并注明 V4L2 ioctl 在同一 fd 上跨线程是可调用但需要记账原子性；`available()` 定义为原子读。另外写明 `FrameLease: Send + !Sync`。

**证明测试**：`assert_send::<FrameLease>()` 不够（E1 说明它恒过）。用 §9.3 的 10k 往返测试**换成断言精确计数**：N 次 lease/drop 后 `qbuf_calls == N && available() == count`；用 `Cell` 实现时该测试在 debug 下就已经红（E2 的 949836/1000000）。再加 `static_assertions::assert_not_impl_any!(FrameLease: Sync)`。

### B3. `stop()` 与 fd 所有权未定义：`Arc` 只保住了 `Drop` 路径，`stop()` 可以合法地 munmap
`interfaces.md:106-112`（配 `86`）

文档说"Drop 顺序由 `Arc` 保证：租约先于池析构"——这句对 `PoolInner::drop` 成立，我不反对。漏的是另一条路径：`FrameSource::stop(&mut self)` 的语义完全没写。`stop()` 里做 `STREAMOFF` + `munmap` + `close(fd)` 完全符合当前文字，而此时另一个线程可能正持有一个 `FrameLease`（§3 的整个前提就是 lease 可以活在别处）→ **use-after-unmap**；即使不 munmap，`BufferPool::new(fd: RawFd, …)` 借的是一个裸 fd，`stop()` 关掉 fd 后 lease 的 `Drop` 里 QBUF 得到 `EBADF`，而 `Drop` 无处返回错误，缓冲区就此静默丢失。

**改法**：三句话必须写进 §3：①`BufferPool` **拥有** fd（`dup` 或 `OwnedFd`，签名改为 `new(fd: impl Into<OwnedFd>, …)` 或明确 `dup`）；②`stop()` 只做 `STREAMOFF`，**不 munmap、不 REQBUFS(0)、不关 fd**，munmap 只发生在 `PoolInner::drop`；③`stop()` 之后 `next_frame` 返回什么（建议 `Err(CaptureError::Stopped)`，不是 `Ok(None)`）、此时 `FrameLease::drop` 的 QBUF 允许成功（回到队列）但失败要计入 `stats`。

**证明测试**：mock backend 记录 `munmap_calls`；持有一个 lease，调 `stop()`，断言 `munmap_calls == 0` 且仍能读到 `planes()` 内容；再 drop lease，断言 `qbuf` 被调用一次、`return_errors == 0`；最后 drop 池，断言 `munmap_calls == 1`。当前契约下"stop 里 munmap"也能过编译、过 §9 现有测试。

### B4. 池容量 8 + 队列深度 8 自相矛盾，且丢帧计数看不到驱动侧丢帧
`interfaces.md:117-118`

line 118 写"池容量 8，队列深度 8，满了按 §5 丢帧并计数"。V4L2 的 buffer 是**同一批**：应用持有 8 个 lease 时驱动手上 0 个 QBUF，它下一秒就采不到帧——`next_frame` 不是"不阻塞"，而是必然超时等消费者还一个。所以"队列深度 8"永远达不到，实际行为是**采集被编码端节流**，表现为 fps 抖动而不是 `dropped++`；而真正的驱动侧丢弃（ISP/内核在无可用 buffer 时丢帧）在应用里完全不可见，`record.event` 的 `dropped` 会系统性少报（`docs/design/ipc.md:33` 的事件与 `index.json.frames_dropped` 都靠它）。

**改法**：写死不变式 `count >= queue_depth + encoder_in_flight + 2`（换成本板：`count=8, queue_depth≤4`），并把 §5 那句悬空引用换成真实策略；`dropped` 定义为 `id`（`v4l2_buffer.sequence`）的缺口累计 + 队列溢出，两者分别计数。

**证明测试**（mock 驱动，模型：只有 QBUF 过的 buffer 才能产出帧）：按 `count=8, depth=8` 跑 §3 的接线，断言 1000 帧内 `next_frame` 超时次数为 0 —— 当前配置下必然 >0；把 depth 降到 4 后转绿。缺口检测：mock 产 `sequence = 0,1,2,5`，断言 `dropped == 2`。

### B5. `flush()` 在 drain 之前归还输入，正是会写坏每一段录像结尾的写法
`interfaces.md:137`

"发 EOS 后把 FIFO 里剩余 lease 全部归还，再返回；期间 `poll()` 允许返回 `None`。" 但 §3.1 自己说"`submit()` 返回不代表硬件读完，`encode_put_frame` 之后硬件在任务里读输入"。EOS 之后 VEPU2 仍在把 FIFO 里的帧编完；此刻 Drop lease → QBUF → 驱动立刻用新帧覆写同一块内存，**最后 N 帧（N = FIFO 深度）被撕裂**。这不是理论问题，是每次 `record.stop` 都会发生一次。而且"期间 poll 允许返回 None"与"把剩余 lease 全部归还再返回"在语义上互斥：若已 drain 完，poll 不会返回 None；若会返回 None，说明没 drain 完却已经归还了。

**改法**：`flush(deadline) -> Result<FlushStats, EncodeError>`：发 EOS → 循环 `poll()` 直到拿到 EOS 标记（或 deadline）→ **每收到一个 packet 才 Drop 对应 lease** → 结束后 FIFO 里若有残留，计入 `lifecycle_errors` 并返回 `LifecycleMismatch`。`submit_eos` 之后禁止 `submit`（写进契约）。

**证明测试**（mock 编码器）：mock 在"吐出该帧 packet"之前把对应 buffer 标记为 `being_read`；断言在 `flush()` 返回前**没有任何** lease 在 `being_read` 期间被归还。用当前实现（submit_eos 后立刻全部归还）该测试第一次运行即红。

### B6. "一个 packet 释放 FIFO 头部一个 lease" 无法自证，且 `poll()` 根本检测不出它不成立
`interfaces.md:131-136`

两个独立缺陷：

1. **检测点错了**。`lifecycle_errors` 要抓的是"少一个 packet"。`poll()` 只会在拿到 packet 时执行，而"少 packet"的信号是"该来的没来"——poll 无法区分"没来"与"还在编"；能观察到它的最早时刻是 EOS/`flush`。文档却写"若不成立，`poll()` 返回 `LifecycleMismatch`"，这条实现不出来。
2. **配对靠 FIFO 头是丢信息的**。packet 里已经有 `pts`（line 165），MPP 会把输入的 pts 原样回吐（`docs/research/mpp-hw-encode.md` §5.4 `mpp_frame_set_pts`）。按头释放意味着"编码器跳过了第 k 帧"（限码率下的跳帧是正常行为 [INFERENCE]）会让后续每一帧的归还都错位一格，到 EOS 时永远剩 1 个 lease → 每次低码率录制都报 `E_ENC_LIFECYCLE` 并进 `Fault`，把正常行为判成故障。

**改法**：`EncodedPacket` 增加 `frame_id: u64`（= 输入 `FrameLease.id`），编码器按 id 归还，`pts` 作为第二校验；缺失 id 累计到 `stats.skipped`（正常丢帧），**只有 `flush` 结束时仍有未归还的 lease 才 `LifecycleMismatch`**。

**证明测试**（mock）：5 次 submit、mock 故意只吐第 1/2/4/5 个 packet，断言 `packets == 4, skipped == 1, in_flight == 1` 且**无** `LifecycleMismatch`；另一条：mock 吐一个 `frame_id` 不在 FIFO 里的 packet，断言立刻 `LifecycleMismatch`（这正是文档现在抓不到的错）。

### B7. `FrameLease` 没有任何取内存/dmabuf 的途径，而 `mmap` 与"MPP 零拷贝"互斥
`interfaces.md:86, 92-101, 179`

`planes: FramePlanes` 私有，注释是"(offset, len)，不含裸指针"；§3.2 line 148 又提到一个 **`planes()` 方法，它不存在**。于是：`VideoEncoder::submit(frame: FrameLease)` 拿到 frame 之后，**没有任何 API 能拿到数据或设备句柄**，编码器无法实现。更要命的是内存类型：`new` 的注释写 `REQBUFS + QUERYBUF + mmap`，而 MPP 零拷贝导入需要 dma-buf fd（仓库自己的调研：`mpp-hw-encode.md` §5.4 `MPP_BUFFER_TYPE_EXT_DMA` + `io-mode=4`；phase 4 注释"如果驱动不支持，改 mmap，但零拷贝会丢失"）。v1 这句话本来是对的——diff 里被删掉的那行是"**默认不复制，MPP 直接消费 dmabuf**"——v2 换成 mmap 且没有补回任何句柄，属于**本次补丁引入的回归**。

**改法**：二选一并写死：①`REQBUFS(V4L2_MEMORY_MMAP)` + `VIDIOC_EXPBUF` 得到每 buffer 的 dma-buf fd，`FrameLease::dmabuf(&self) -> BorrowedFd<'_>`；②直接 `V4L2_MEMORY_DMABUF`，自己用 dma-heap 分配后入队。同时给出 `fn luma(&self) -> &[u8]` / `fn chroma(&self) -> &[u8]`（生命周期绑 `&self`，见 E3），以及 `frame_geom()`（含 stride/vstride）。

**证明测试**：板上——对 `/dev/video0` 每个 buffer 调 `VIDIOC_EXPBUF` 成功、`mpp_buffer_import(EXT_DMA)` 返回 `MPP_SUCCESS` 且无 memcpy（`mpp_buffer_get_size` 与 `sizeimage` 相等）；host——mock backend 断言编码器 mock 收到的句柄与池 slot 一一对应（不复制）。当前契约下 host 连"怎么拿到字节"都写不出来，测试无从下手。

### B8. §3.2 只定义了水平 stride，MPP 还需要 ver_stride；2592×1944 直通会在映射末尾外读 31,104 字节
`interfaces.md:139-152`（配 `56-63`）

`total = stride*h*3/2` 与 `sizeimage = 7_558_272` 的等式在本板成立，说明驱动给的是**紧凑 1944 行**。但 MPP 的输入描述是 `(hor_stride, ver_stride)`，且仓库自己的调研明确写 `prep:ver_stride = 1080/1088（16 对齐）`、"`hor_stride`/`ver_stride` 不能想当然等于宽高，MPP 要求 16 对齐"（`mpp-hw-encode.md` §5.2、phase 3 第 2 条）。1944 不是 16 的倍数（`align16(1944)=1952`）：按 vstride=1952 寻址时 buffer 需要 `2592*1952*3/2 = 7,589,376` 字节，**比 sizeimage 多 31,104 字节**，luma 平面之外还要读 8 行；chroma 平面起点也从 `stride*h` 变成 `stride*vstride`。§3.2 的公式在这种情形下是错的，而 §5 line 193 恰好把 2592×1944 直通列为"实测达标"的主路径。H16 能跑通说明当时用的是 MPP 自己分配/对齐过的 buffer，不是这版契约描述的 V4L2 mmap 直通 [INFERENCE]。

**改法**：`nv12_luma_len`/`nv12_total` 改成收 `(stride, vstride)`，`FrameLease` 带 `vstride`；启动时断言 `sizeimage >= nv12_total(stride, vstride)`，不满足就 `BadStride`/`UnsupportedFormat` 而不是继续；§3.2 明写零缩放路径传给 MPP 的 `ver_stride` 是 `h` 还是 `align16(h)`，以及由谁保证 buffer 够长。

**证明测试**：`assert!(sizeimage >= nv12_total(2592, 1952))` —— 本板驱动值 7,558,272 会**红**，这正是要在冻结前拍板的结论；板上另一条：2592×1944 编 1 帧后校验末尾 16 行不是垃圾（与本地裁剪参考比对）。

### B9. §5 的验收门槛"1080p 缩放 + 编码 ≥30 fps"与仓库自己的实测 27.0 fps 冲突
`interfaces.md:196-197`（配 `docs/architecture.md` H12/H14）

H12：相机→H.264 1080p 持续 **27.0 fps**（300 帧/11.13 s），H14：四种分辨率（含 1920×1080）`--stream-mmap` 全部 **27–28 fps**。§5 把默认 `encode.geom` 定成 1920×1080，同时又要求 RGA 缩放路径达到 ≥30 fps——这条门槛在**什么都不做**的基线上就已经不达标，追加一次 RGA blit 只会更低。冻结后 `xtask smoke` 必然红，然后要么改门槛要么改默认分辨率，两者都是契约变更。

**改法**：门槛改为可判定的绝对量（`fps_actual ≥ 25` 且 `frames_dropped == 0`），或把默认 `encode.geom` 改成 2592×1944（H16 实测 30.045 fps 的那条路径），笔记本上再确认 RGA 是否值得引入。

**证明测试**：`xtask smoke` 读 `index.json.fps_actual` 与文档门槛比对；用 H12 的 27.0 代入即红。另一条 `action-config` 单测：默认配置 `(1920,1080,30)` 必须能通过"§5 声明可达"的检查。

### B10. §3/§4 的代码块不可编译：`MonoTime`/`Geometry`/`PixelFormat` 被本补丁删了定义却仍在用
`interfaces.md:92-97`（`MonoTime`/`Geometry`/`PixelFormat`/`FramePlanes`/`BufferIndex`/`SourceInfo`/`CaptureError`/`EncodeError`/`EncoderStats`/`Codec`）

diff 显示 v1 的 `MonoTime`（含"纳秒、单一 CLOCK_MONOTONIC 时间域"）、`Geometry`、`PixelFormat`、`TsSource`、`Timebase::from_driver` 整块被删，而新文本继续使用这些类型（`FrameLease.pts`、`EncoderConfig.geom`、`EncodedPacket.pts`、`ImuSample.t`）。同时驱动时间戳→单调时钟的换算契约（offset 取中值、写进 `index.json`，见 `docs/architecture.md` §6.2）**现在没有任何文档拥有**：`docs/design/README.md` 把时钟域派给 `timebase.md`，而它是"待写"，且 interfaces.md 连一句链接都没留。§9 的 host 测试也因此无法编译。

**改法**：在 §2（`action-contract`，零依赖）补回 `MonoTime(pub u64)`、`Geometry{w,h}`、`PixelFormat`、`FrameGeometry{ w, h, stride, vstride }`，并在 §3 加一句"`pts` 的时钟域与 `from_driver` 规则见 timebase.md（冻结前必须有内容，否则以本页为准）"。

**证明测试**：把 §3+§4 的 rust 代码块原样粘进一个空 crate 后 `cargo check` → `error[E0412]: cannot find type MonoTime in this scope`（当前即如此）。这条测试顺便可以做成 `xtask docs` 的"代码块可编译"门禁，与 ipc.md 的"JSON 块 = 夹具"同构。

---

## Should fix

**S1. §3.2 有两个互斥的 chroma 偏移来源**（`interfaces.md:98, 144`）。`FramePlanes` 说是"（offset,len）指向池映射"，而 §3.2 又把 `chroma = stride*h` 写成定义。若驱动给的是 `V4L2_PIX_FMT_NV12M` 或带 plane padding，两者不等，`nv12_total` 作为 `sizeimage` 的等值校验就会把正常格式判成错。改法：偏移以 QUERYBUF 的 `planes[].m.offset/length` 为唯一真源，§3.2 的公式降级为"期望值，启动时断言相等，否则用驱动值"。测试：mock 给 chroma offset = stride*h + 4096，断言 lease 用驱动值且不报错；`nv12_total` 只做下界断言。

**S2. §5 的"第二个池"用 §3 的 `BufferPool` 表达不出来**（`interfaces.md:194-195`）。`new(fd)` 做的是 `REQBUFS/QUERYBUF/mmap`（V4L2 专用），而 RGA 输出 buffer 是 dma-heap/drm 分配。改法：把"池"抽成 `FrameBufferPool`（`alloc → lease → free`），V4L2 池与 RGA 池各一个实现；这也正好是 §9 需要的 mock 注入点（见 S3）。测试：RGA 池 `MockAlloc` 分配 2560×1440 后交给编码器 mock，断言 `stride/vstride` 由分配器决定而非从 V4L2 `bytesperline` 读。

**S3. §9 全部测试都没有注入点：`BufferPool::new(RawFd)` 在 host 上跑不起来**（`interfaces.md:253-260`）。§9.2 要"mock 记录 QBUF 调用次数"，§9.4 要"mock 计数断言 munmap"，但 `BufferPool` 的唯一构造器直接做 ioctl。WSL 是 Linux，`cfg(target_os="linux")` 不会把这些测试排除，只会让 `/dev/video0` 的 ioctl 返回 `ENOENT`；在 Windows 侧编译时模块整个消失，`cargo test` 静默 0 个用例也算绿。改法：`trait PoolBackend { fn reqbufs/querybuf/mmap/qbuf/dqbuf/munmap }`，`BufferPool<B: PoolBackend>` 或 `BufferPool { backend: Box<dyn PoolBackend> }`，记账放 `PoolCore`（纯状态机，无 OS 调用），`V4l2Backend` 只在 `#[cfg(target_os="linux")]` 里。测试：在 WSL 里 `cargo test --target x86_64-unknown-linux-gnu pool` 用 `MockBackend` 跑完 §9.1/2/3/4/5，全程不碰 `/dev/video0`；`PoolCore` 另有一组纯单测（无 `cfg`，Windows 也能跑）。

**S4. §9.4 是空测试**（`interfaces.md:258`）。"线程 join 后池 Drop，无 use-after-unmap" 在 `Arc` 语义下**不可能失败**——租约持 `Arc`，池析构不到。它验证不变量"我们已经用 `Arc` 了"，不是风险点。真正的风险是 `stop()`（B3）。改法：换成"持 lease → 另一线程 `stop()` → 读 lease 内容 → drop lease → drop 池"，并用 mock 断言 `munmap` 只在最后一次 drop 时发生。

**S5. `submit` 的错误路径把 lease 吞掉，无法重试**（`interfaces.md:170`）。`encode_put_frame` 可能瞬时失败（输出超时/输入队列满，调研文档 §5.1 里 `MPP_SET_OUTPUT_TIMEOUT` 就是为这个存在的）；`Result<(), EncodeError>` 意味着帧被静默丢弃且调用方无从补救，在慢消费者下会变成系统性掉帧。改法：`-> Result<(), (FrameLease, EncodeError)>` 或 `submit(&mut self, frame: &mut Option<FrameLease>)`。测试：mock 对第 3 次 submit 返回 `E_AGAIN`，断言 lease 被还回调用方且 `dropped == 0`、重提后 `submits == 5`。

**S6. 白名单里的 2560×1440 没有对应的 `ENC_FPS_CEIL_*`**（`interfaces.md:190` 对比 `65-68`）。§5 最后一行要求"`fps ≤ ceiling`，否则 `E_CFG`"，但只有 5MP/1080P/720P 三个常量，2560×1440（3.69MP）无门槛 → 校验分支写不出来，实现阶段只能自己发明一个数。改法：补 `ENC_FPS_CEIL_1440P`（需实测）或从白名单删掉该项。测试：对白名单做表驱动单测，断言每一项都能查到 ceiling；当前对 2560×1440 直接 panic/失败。

**S7. 4:3 → 16:9 没有 crop/letterbox 规则，默认配置下每一段录像都是拉伸的**（`interfaces.md:190-195`）。`sensor 2592×1944`（1.333）→ 默认 `1920×1080`（1.778），§5 只说"插入 RGA 缩放"，没说裁边还是拉伸；而 `preview.geom` 明确写了"可 letterbox"，说明作者知道有这个问题，只是漏了录制路径。改法：在 §5 加一行 `fit(源, 目标) -> {crop, scale}` 策略（默认 center-crop 还是 pad），并让 RGA 的 src rect 由它决定。测试：`fit(4:3, 16:9)` 的单测——断言输出的像素宽高比与源一致（±1%），或用 16×12 棋盘断言落盘画面的圆不是椭圆。

**S8. `BufferPool::new` 缺 format，且 `FrameLease.stride/geom` 的来源是断的**（`interfaces.md:86, 94-97`）。`REQBUFS` 必须在 `S_FMT` 之后，`bytesperline/sizeimage` 来自 `G_FMT`；但 `new(fd, count)` 没有 format 参数，而 format 是在 `FrameSource::start(geom, fps)` 里才设置的。结果 lease 的 `stride`/`geom` 只能从"别的地方"抄，池和帧可能不一致（真出现时是静默的错行解码）。改法：`new(fd, &Format{ pix, w, h }, count)`，池持有 `G_FMT` 的结果，`start()` 只做 `S_FMT` + `STREAMON`，并用 mock 断言 ioctl 顺序 `S_FMT → REQBUFS → QUERYBUF → STREAMON`。

---

## Nit

- **N1** `available()` 语义未定义（`interfaces.md:89`）：是 Free 数，还是"Free + 已 QBUF 但未出队"？§9.1/§9.2 都依赖它，建议写成"当前处于 Free 状态、可立即 lease 的槽位数"。
- **N2** `FrameLease.id` 的赋值规则没写（`interfaces.md:93`）。建议直接钉成 `v4l2_buffer.sequence`——它是唯一能让 B4 的驱动侧丢帧可见的量。
- **N3** `EncodedPacket` 是否必须回吐输入 `pts`（`interfaces.md:165`）没写；B6 的按 id/pts 配对依赖它，建议写成硬约束（`mpp_frame_set_pts` 原样回吐），否则 `frames.csv` 与视频帧无法对齐。
- **N4** `nv12_total` 的 `const _: () = assert!(... == sizeimage)`（`interfaces.md:61`）把"本板测得的偶然相等"固化成等式；换成 `sizeimage >= nv12_total(...)` 就不会在换 sensor/换驱动时误报（与 B8 的 vstride 一起改）。
- **N5** `stop(&mut self)` 无返回值、无错误通道（`interfaces.md:111`）：`STREAMOFF` 失败或 QBUF 失败只能进日志，而 ipc.md §6 要求错误进 `record.event`/`index.json.errors`。建议 `stop(&mut self) -> Result<(), CaptureError>`。
- **N6** `docs/design/README.md` 把"步幅对齐"派给 `capture-encode.md`（待写），而 §3.2 现在自称"精确定义"——违反"一机制一页"这条自己的门禁；要么把 §3.2 标为占有者并同步改 README，要么只留一句+链接。
- **N7** §9 第 5 条标题写"解码/编码配对"（应为编码），且断言"返回 `E_ENC_LIFECYCLE`"跨越了层：`poll` 返回 `EncodeError`，`1007` 是 `action-ipc` 的线上码，`action-core` 的测试应断言 `EncodeError::LifecycleMismatch`。

---

## 如果照现在这样冻结，最可能在第几步炸

1. **写池的第一天**：B1 + B10。`dqbuf` 拿不到 `timestamp/sequence`，`FrameLease.pts` 填不进去；同时 `MonoTime`/`Geometry` 没有定义，代码块原样落地编译不过。这一步就会被迫改签名 → 冻结失效。
2. **接编码器**：B7。lease 里没有 dmabuf fd 也没有字节访问器，唯一出路是自己 `memcpy` 进 MPP buffer——正好违反 §3.2 "重新打包算违约"，并且在 2592×1944 上直接吃掉 H16 的 30 fps 余量。
3. **第一次 1080p 录制**：B8 + S7。`vstride` 未定义导致末尾 8 行外读（当场 SIGSEGV 或花屏），4:3→16:9 拉伸肉眼可见。
4. **第一次 `record.stop`**：B5。最后几帧被驱动覆写，MKV 尾部花掉，而 `index.json` 写 `complete: true`。
5. **跑 §9**：S3 + S4。`BufferPool::new(fd)` 在 WSL 没有 `/dev/video0`，`available()` 在 `Cell` 记账下永远对不上，§9.4 又因为 `Arc` 恒绿——三个测试证明了零个不变量。
6. **上板跑 `xtask smoke`**：B9。文档门槛 30 fps 对实测 27.0 fps，红；随后要么改门槛要么改默认分辨率，两者都是契约变更，回到第 1 步。

最短出路：先把 §3 的池 API 收成 `dqbuf -> FrameLease` + `PoolBackend` 注入 + 池拥有 fd/`stop()` 不 munmap，再把 §3.1 的配对从"FIFO 头"换成"按 id"，最后补 `vstride` 与类型定义——这四项一改，B1–B8 与 S1–S5 里的绝大多数症状会一起消失，剩下的只是填数字。