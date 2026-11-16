# H13：rkisp MIPI 时间戳语义（已关闭）

- 日期：2026-11-16 · 板：`k@192.168.31.129` · 内核 4.19.232 aarch64 · Ubuntu 20.04.6
- 设备：`/dev/video0`，驱动 `rkisp_v5`，card `rkisp_mainpath`，`Video Capture Multiplanar`
- 工具：`tools/hil/h13_probe.py`（纯 ctypes；板上无 gcc，也没有 `linux/videodev2.h`）

## 结论

| 问题 | 答案 | 证据 |
|---|---|---|
| 时钟域 | `CLOCK_MONOTONIC` | `flags=0x00002001`：`V4L2_BUF_FLAG_TIMESTAMP_MONOTONIC`（0x2000）置位，`TIMESTAMP_MASK` = 0x2000 |
| 语义 | **EOF（帧结束）** | `V4L2_BUF_FLAG_TSTAMP_SRC_MASK`（0x10000）未置位 = `TSTAMP_SRC_EOF` |
| 与单调钟的偏差 | 同域，`clock_offset_ns = 0` | DQBUF 时 `timestamp` 落后 `CLOCK_MONOTONIC` 仅 1.1–2.2 ms（处理延迟，不是钟差） |
| 采集时刻换算 | `t_capture ≈ t_eof − 1/fps` | EOF 语义的定义；1/fps = 33.3 ms @30 fps，比 2 ms 的处理延迟大一个量级 |

`record.event` / `index.json.timestamp_semantics` 因此写 **`"eof"`**，不是 `"sof"`。H2 的 USB/UVC 结论（`ts-src-soe`）不能套到 rkisp 上——这正是 H13 存在的理由。

## 原始输出（1920x1080，4 buffers，8 帧）

```text
negotiated: 1920x1080 fourcc=0x3231564e field=1 stride=1920 sizeimage=3110400
seq=    0 used= 3110400 flags=0x00002001 tstype=MONOTONIC src=EOF delta_ms=   2.15
seq=    1 used= 3110400 flags=0x00002001 tstype=MONOTONIC src=EOF delta_ms=   1.57
seq=    2 used= 3110400 flags=0x00002001 tstype=MONOTONIC src=EOF delta_ms=   1.69
...
```

2592x1944（5 帧）：`stride=2592 sizeimage=7558272`，同样 `flags=0x00002001`，`delta_ms` 1.11–2.07。

## 同时验证的契约常量（硬件）

| 几何 | `bytesperline` | `sizeimage` | `action-contract` |
|---|---|---|---|
| 1920x1080 NV12 | 1920 | 3,110,400 | `SIZEIMAGE_1080P` ✅ |
| 2592x1944 NV12 | 2592 | 7,558,272 | `SIZEIMAGE_2592X1944` ✅ |

两种模式都 `vstride == h`、`stride == w`，与契约"实测值，不是契约本身"的表述一致。

## 写 V4L2 绑定时必须知道的板级事实

1. **节点是 multiplanar**：`V4L2_BUF_TYPE_VIDEO_CAPTURE_MPLANE`（9）。用单平面 `VIDEO_CAPTURE` 一律 `EINVAL`。
2. **本内核 `sizeof(struct v4l2_format) = 208`**；`G_FMT/S_FMT` 的 ioctl size 必须是 208。
3. `plane_fmt[0]` 的字段在这个 vendor 头里不是 mainline 顺序：实测 `sizeimage` 在 `v4l2_format` 偏移 **28**、`bytesperline` 在 **32**（mainline 是 4 字节顺序不同）。
4. `v4l2_buffer` 88 B + `v4l2_plane` 64 B 数组；`buf.length` 必须等于平面数（1），`buf.m.planes` 指向 plane 数组。
5. ioctl 方向容易错：`VIDIOC_G_FMT` 是 `_IOWR`（不是 `_IOR`），`VIDIOC_STREAMON/OFF` 是 `_IOW(..., int)`（size=4，不是 `_IO`）。方向/大小不对时内核返回 `EINVAL`，看起来像"驱动不支持"。
6. 板上无 `gcc`、无内核头文件，所以探针用 ctypes；Rust 绑定侧同样不能依赖板上的头文件做 bindgen（需 vendored 常量）。

## 复现

```bash
scp tools/hil/h13_probe.py k@<board>:/tmp/
ssh k@<board> 'python3 /tmp/h13_probe.py /dev/video0 1920 1080 8'
ssh k@<board> 'python3 /tmp/h13_probe.py /dev/video0 2592 1944 5'
```

前提：`k` 在 `video` 组（`/dev/video0` 是 `0660 root:video`）。

## 本仓 V4L2 绑定在板上的实测（2026-11-16）

`examples/capture_probe`（static musl）以 `k` 用户直接运行，**不需要 sudo**（`k` 在 `video` 组，
`/dev/video0` 是 `0660 root:video`）：

```text
./capture_probe /dev/video0 1920 1080 30 20
started 1920x1080 fps=30 format=Nv12 stride=1920 vstride=1080
frame id=0 pts=2307026703000ns luma=2073600B chroma=1036800B
done: 20 frames in 0.685s = 29.21 fps; stats=PoolStats { dequeued: 20, returned: 20, return_errors: 0, gap_frames: 0 }

./capture_probe /dev/video0 2592 1944 30 10
started 2592x1944 fps=30 format=Nv12 stride=2592 vstride=1944
done: 10 frames in 0.354s = 28.29 fps; stats=PoolStats { dequeued: 10, returned: 10, return_errors: 0, gap_frames: 0 }
```

平面字节数正好等于 `sizeimage`（1080p 3,110,400；5MP 7,558,272），`pts` 单调递增，
`gap_frames`/`return_errors` 为 0。即：**Pool 的 mmap 槽位、按 `index` 租还、QBUF 归还链路
在真实驱动上闭环**（host 侧已有并发与 poison 用例覆盖）。
