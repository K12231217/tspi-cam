# 录制格式与完整性

- 状态：v1（2026-11-16，为 IPC `IndexSummary` 解冻）· 拥有者：本文 = `rec-*/` 目录布局、`index.json`、
  CSV 列、完整性与丢帧计数
- 依赖：[ipc.md](ipc.md) §3.2 的 `IndexSummary` 引用本文的字段表。

## 1. 目录布局

```text
/userdata/rec/rec-20261116-141230-3f2a/
├── video.mkv        # H.264 / H.265（MPP VEPU2 硬编）
├── frames.csv       # frame_id,t_mono_ns
├── imu.csv          # t_mono_ns,gx,gy,gz,ax,ay,az   (rad/s, m/s²)
└── index.json       # 最后写；唯一"完整"标志
```

- 目录名 = `rec-<YYYYMMDD-HHMMSS>-<4 hex>`，`<4 hex>` 是启动时随机后缀，避免同一秒两次录制冲突。
- `rec_id` 就是目录名去掉 `rec-` 前缀之后的部分（`20261116-141230-3f2a`）；`record.get/delete` 用它。
- 录制目录只追加，不在录制中改名。

## 2. `IndexSummary`（IPC 的 `record.stop/get/list` 结果）

```json
{
  "rec_id": "20261116-141230-3f2a",
  "complete": true,
  "started_mono_ns": 88123456789,
  "ended_mono_ns": 208123456789,
  "duration_ms": 120000,
  "frames": 3600,
  "imu_samples": 24960,
  "dropped": {"gap_frames": 0, "queue_dropped": 0, "imu_dropped": 0},
  "bytes": 481846567,
  "codec": "h264",
  "width": 2592,
  "height": 1944,
  "fps_actual": 30.045,
  "imu_rate_actual_hz": 208.0,
  "errors": []
}
```

| 字段 | 类型 | 说明 |
|---|---|---|
| `rec_id` | string | 目录标识 |
| `complete` | bool | **唯一完整性标志**；缺 `index.json` 或为 `false` 即"未完成" |
| `started_mono_ns` / `ended_mono_ns` | u64 | `CLOCK_MONOTONIC` 纳秒，录制起止（与 `frames.csv` 同域） |
| `duration_ms` | u64 | `(ended - started) / 1e6` |
| `frames` / `imu_samples` | u64 | 写入成功的数量（不是要求数量） |
| `dropped` | object | **两个来源分开**：`gap_frames`（`v4l2_buffer.sequence` 缺口，驱动侧丢）、`queue_dropped`（应用队列溢出）、`imu_dropped` |
| `bytes` | u64 | 目录总字节数 |
| `codec` | `"h264"`/`"h265"` | 与 `video.mkv` 一致 |
| `width`/`height` | u32 | 编码分辨率（`encode.geom`，不是 sensor 分辨率） |
| `fps_actual` | f64 | `frames / duration`（实测值，不是目标 30） |
| `imu_rate_actual_hz` | f64 | `imu_samples / duration` |
| `errors` | `[{class, code, message, at_mono_ns}]` | 与 `record.event` 同源；空数组不是 null |

`record.list` 的 `{records:[IndexSummary], next?}` 与 `record.get` 的 `IndexSummary` 用同一结构；
列表按 `started_mono_ns` 降序。

## 3. CSV

- `frames.csv`：`frame_id,t_mono_ns`。`frame_id` 必须等于 `video` 里该帧的 PTS 来源序号
  （`v4l2_buffer.sequence`），一行一帧，**不允许用固定 FPS 推算**。
- `imu.csv`：`t_mono_ns,gx,gy,gz,ax,ay,az`，单位 rad/s 与 m/s²；每行一个样本，`t_mono_ns` 是
  `t_spi_read`（M2 起可换 `t_drdy`，届时在 `index.json` 标注）。
- 两个 CSV 只追加、不改写；行数必须等于 `index.json` 的 `frames` / `imu_samples`。

## 4. 写出顺序与崩溃语义

1. `video.mkv`、`frames.csv`、`imu.csv` 边录边写（各自独立线程/句柄）。
2. 停止时：停生产 → Drain 队列 → `fsync` 三个文件 → 写 `index.json.tmp` → `fsync` →
   `rename(index.json.tmp, index.json)` → `fsync` 目录。
3. **断电/崩溃**：没有 `index.json`（或 `complete=false`）就是未完成记录，PC 端不得当完整数据用；
   `frames.csv`/`imu.csv` 里的行仍然有效，但缺少结束时间与计数，取用时以文件行数为准并标注。
4. `record.delete` 只删完整目录（`complete=true` 或用户 `confirm` 明确接受半成品）；
   删除先 `rename` 到 `rec-…-deleting` 再递归删除，避免半删状态被列表读到。

## 5. 状态机（与 ipc.md §3.5 对应）

```text
idle ──record.start──▶ starting ──设备就绪──▶ recording
  ▲                        │                      │
  │                        └─失败─▶ idle(E_HW/E_CFG)│
  └────finalize ok──────── stopping ◀──record.stop──┘
                              │
                              └─finalize 失败─▶ 记录 complete=false + record.event(E_STORAGE)
```

- `starting` 不出现在 IPC 的 `state` 里：`record.start` 在 `Operation` lane 内同步等到设备就绪才返回
  `{rec_id,state:"recording"}`；失败直接返回 `E_HW`/`E_CFG`，不会留下半个目录（失败时删空目录）。
- `stopping` 期间的 `record.status` 返回 `state:"stopping"` + 已写计数；`record.start/stop` 返回
  `E_STATE{reason:"busy_stopping"}`。
- `record.stop` 返回的 `IndexSummary.complete` 必须是 `true` 才算正常结束；`false` 时
  `record.event` 同时推送 `E_STORAGE`。

## 6. 校验（`xtask` 与 PC 端共用）

1. 行数：`frames.csv` 行数 == `frames`，`imu.csv` 行数 == `imu_samples`；
2. 单调：两列 `t_mono_ns` 各自严格递增；
3. 首尾：`started_mono_ns <= 首行 <= 末行 <= ended_mono_ns`；
4. 容器：`ffprobe` 能读出 `codec/width/height`，且与 `index.json` 一致；
5. 丢帧自洽：`gap_frames` 等于 `frames.csv` 中 `frame_id` 缺口的累计（抽样校验最近 1000 帧即可）。
